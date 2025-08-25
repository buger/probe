use crate::simd_ranking::{SimdBm25Params, SparseDocumentMatrix};
use ahash::{AHashMap, AHashSet};
use probe_code::search::elastic_query::Expr;
use probe_code::search::tokenization;
use rayon::prelude::*;
use rust_stemmers::{Algorithm, Stemmer};
use std::sync::OnceLock;

// Replace standard collections with ahash versions for better performance
type HashMap<K, V> = AHashMap<K, V>;
type HashSet<T> = AHashSet<T>;

/// Maps unique query tokens (Strings) to unique u8 indices
pub type QueryTokenMap = HashMap<String, u8>;

/// Represents the result of term frequency and document frequency computation
pub struct TfDfResult {
    /// Term frequencies for each document, using u8 index for query tokens
    pub term_frequencies: Vec<HashMap<u8, usize>>,
    /// Document frequencies for each term (remains String-based for IDF)
    pub document_frequencies: HashMap<String, usize>,
    /// Document lengths (number of tokens in each document)
    pub document_lengths: Vec<usize>,
}

/// Parameters for document ranking
pub struct RankingParams<'a> {
    /// Documents to rank
    pub documents: &'a [&'a str],
    /// Query string
    pub query: &'a str,
    /// Pre-tokenized content (optional)
    pub pre_tokenized: Option<&'a [Vec<String>]>,
}

/// Returns a reference to the global stemmer instance
pub fn get_stemmer() -> &'static Stemmer {
    static STEMMER: OnceLock<Stemmer> = OnceLock::new();
    STEMMER.get_or_init(|| Stemmer::create(Algorithm::English))
}

/// Tokenizes text into lowercase words by splitting on whitespace and non-alphanumeric characters,
/// removes stop words, and applies stemming. Also splits camelCase/PascalCase identifiers.
pub fn tokenize(text: &str) -> Vec<String> {
    tokenization::tokenize(text)
}

/// Preprocesses text for search by tokenizing and removing duplicates
/// This is a lighter version that doesn't include filename tokens
pub fn preprocess_text(text: &str) -> Vec<String> {
    tokenize(text)
}

/// Preprocesses text with filename for search by tokenizing and removing duplicates
/// This is used for filename matching - it adds the filename and its directory structure to the tokens
pub fn preprocess_text_with_filename(text: &str, filename: &str) -> Vec<String> {
    let mut tokens = tokenize(text);
    let filename_tokens = tokenize(filename);
    tokens.extend(filename_tokens);
    tokens
}

/// Computes the average document length.
pub fn compute_avgdl(lengths: &[usize]) -> f64 {
    if lengths.is_empty() {
        return 0.0;
    }
    // Convert to f64 before summing to prevent potential integer overflow
    // when dealing with very large documents or a large number of documents
    let sum: f64 = lengths.iter().map(|&x| x as f64).sum();
    sum / lengths.len() as f64
}

// -------------------------------------------------------------------------
// BM25 EXACT (like Elasticsearch) with "bool" logic for must/should/must_not
// -------------------------------------------------------------------------

/// Parameters for BM25 calculation with precomputed IDF values
pub struct PrecomputedBm25Params<'a> {
    /// Document term frequencies using u8 indices
    pub doc_tf: &'a HashMap<u8, usize>,
    /// Document length
    pub doc_len: usize,
    /// Average document length
    pub avgdl: f64,
    /// Precomputed IDF values for query terms (remains String-based)
    pub idfs: &'a HashMap<String, f64>,
    /// Map from query term string to u8 index
    pub query_token_map: &'a QueryTokenMap,
    /// BM25 k1 parameter
    pub k1: f64,
    /// BM25 b parameter
    pub b: f64,
}

/// Extracts unique terms from a query expression
pub fn extract_query_terms(expr: &Expr) -> HashSet<String> {
    use Expr::*;
    let mut terms = HashSet::new();

    match expr {
        Term { keywords, .. } => {
            terms.extend(keywords.iter().cloned());
        }
        And(left, right) | Or(left, right) => {
            terms.extend(extract_query_terms(left));
            terms.extend(extract_query_terms(right));
        }
    }

    terms
}

/// Precomputes IDF values for a set of terms
pub fn precompute_idfs(
    terms: &HashSet<String>,
    dfs: &HashMap<String, usize>,
    n_docs: usize,
) -> HashMap<String, f64> {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!(
            "DEBUG: Precomputing IDF values for {terms_len} terms",
            terms_len = terms.len()
        );
    }

    terms
        .iter()
        .filter_map(|term| {
            let df = *dfs.get(term).unwrap_or(&0);
            if df > 0 {
                let numerator = (n_docs as f64 - df as f64) + 0.5;
                let denominator = df as f64 + 0.5;
                let idf = (1.0 + (numerator / denominator)).ln();
                Some((term.as_str(), idf))
            } else {
                None
            }
        })
        .map(|(term, idf)| (term.to_string(), idf))
        .collect()
}

/// Generates a mapping from unique query tokens to unique u8 indices.
///
/// This function takes a set of query terms and assigns a unique u8 index to each term.
/// The mapping is used to efficiently represent query terms in memory-constrained contexts.
///
/// # Arguments
///
/// * `query_terms` - A set of unique query terms to map to indices
///
/// # Returns
///
/// * `Result<QueryTokenMap, &'static str>` - A mapping from terms to indices, or an error if there are too many terms
///
/// # Errors
///
/// Returns an error if the number of unique query terms exceeds 256 (the maximum value for u8).
fn generate_query_token_map(query_terms: &HashSet<String>) -> Result<QueryTokenMap, &'static str> {
    // Check if we have too many terms for u8 mapping
    if query_terms.len() > 256 {
        return Err("Query exceeds the 256 unique token limit for u8 mapping");
    }

    let mut token_map = QueryTokenMap::new();
    let mut index: u8 = 0;

    // Sort terms for deterministic mapping (HashMap iteration order isn't guaranteed)
    let mut sorted_terms: Vec<&str> = query_terms.iter().map(|s| s.as_str()).collect();
    sorted_terms.sort();

    // Assign each term a unique index
    for term in sorted_terms {
        token_map.insert(term.to_string(), index);
        index = index.wrapping_add(1); // Use wrapping_add to handle potential overflow safely
    }

    Ok(token_map)
}

/// Optimized BM25 single-token function using precomputed IDF values:
/// tf_part = freq * (k1+1) / (freq + k1*(1 - b + b*(docLen/avgdl)))
fn bm25_single_token_optimized(token: &str, params: &PrecomputedBm25Params) -> f64 {
    // Look up the token's index in the query_token_map
    let Some(&token_index) = params.query_token_map.get(token) else {
        // This query term string doesn't have a u8 mapping? Should not happen if map is correct.
        // Or maybe the term was stemmed differently? Return 0 for safety.
        return 0.0;
    };

    // Get frequency using the u8 index
    let freq_in_doc = *params.doc_tf.get(&token_index).unwrap_or(&0) as f64;
    if freq_in_doc <= 0.0 {
        return 0.0;
    }

    // Use precomputed IDF value (still using string for IDF lookup)
    let idf = *params.idfs.get(token).unwrap_or(&0.0);

    let tf_part = (freq_in_doc * (params.k1 + 1.0))
        / (freq_in_doc
            + params.k1 * (1.0 - params.b + params.b * (params.doc_len as f64 / params.avgdl)));

    idf * tf_part
}

/// Sum BM25 for all keywords in a single "Term" node using precomputed IDF values
fn score_term_bm25_optimized(keywords: &[String], params: &PrecomputedBm25Params) -> f64 {
    let mut total = 0.0;
    for kw in keywords {
        total += bm25_single_token_optimized(kw, params);
    }
    total
}

/// Recursively compute a doc's "ES-like BM25 bool query" score from the AST using precomputed IDF values:
/// - If it fails a must or matches a must_not => return None (exclude doc)
/// - Otherwise sum up matched subclause scores
/// - For "OR," doc must match at least one side
/// - For "AND," doc must match both sides
/// - For a "should" term, we add the BM25 if it matches; if the entire query has no must, then
///   at least one "should" must match in order to include the doc.
pub fn score_expr_bm25_optimized(expr: &Expr, params: &PrecomputedBm25Params) -> Option<f64> {
    use Expr::*;
    match expr {
        Term {
            keywords,
            required,
            excluded,
            ..
        } => {
            let score = score_term_bm25_optimized(keywords, params);

            if *excluded {
                // must_not => doc out if doc_score > 0
                if score > 0.0 {
                    None
                } else {
                    Some(0.0)
                }
            } else if *required {
                // must => doc out if doc_score=0
                if score > 0.0 {
                    Some(score)
                } else {
                    None
                }
            } else {
                // "should" => we don't exclude doc if score=0 here, because maybe it matches
                // something else in an OR. Return Some(0.0 or some positive).
                // The top-level logic ensures if no must in the entire query, we need at least one should>0.
                Some(score)
            }
        }
        And(left, right) => {
            let lscore = score_expr_bm25_optimized(left, params)?;
            let rscore = score_expr_bm25_optimized(right, params)?;
            Some(lscore + rscore)
        }
        Or(left, right) => {
            let l = score_expr_bm25_optimized(left, params);
            let r = score_expr_bm25_optimized(right, params);
            match (l, r) {
                (None, None) => None,
                (None, Some(rs)) => Some(rs),
                (Some(ls), None) => Some(ls),
                (Some(ls), Some(rs)) => Some(ls + rs),
            }
        }
    }
}

// -------------------------------------------------------------------------
// This is your main entry point for ranking. It now does "pure BM25 like ES."
// -------------------------------------------------------------------------
pub fn rank_documents(params: &RankingParams) -> Vec<(usize, f64)> {
    use std::cmp::Ordering;

    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    // 1) Parse the user query into an AST (Expr)
    //    If your code uses parse_query(...) from `elastic_query.rs`, do:
    let parsed_expr = match crate::search::elastic_query::parse_query(params.query, false) {
        Ok(expr) => expr,
        Err(e) => {
            if debug_mode {
                eprintln!("DEBUG: parse_query failed: {e:?}");
            }
            // Instead of silently returning empty results, log a warning even in non-debug mode
            // to ensure errors are visible and can be addressed
            eprintln!("WARNING: Query parsing failed: {e:?}. Returning empty results.");
            // In a future version, consider changing the return type to Result<Vec<(usize, f64)>, QueryError>
            // to properly propagate errors to the caller
            return vec![];
        }
    };

    // 3) Extract query terms, create token mapping
    let query_terms = extract_query_terms(&parsed_expr);

    // Generate query token map (maps each unique query term to a unique u8 index)
    let query_token_map = match generate_query_token_map(&query_terms) {
        Ok(map) => map,
        Err(e) => {
            if debug_mode {
                eprintln!("DEBUG: Failed to generate query token map: {e}");
            }
            eprintln!("WARNING: {e}");
            return vec![];
        }
    };

    if debug_mode {
        println!(
            "DEBUG: Generated query token map with {} entries",
            query_token_map.len()
        );
    }

    // 2) Precompute TF/DF for docs
    let tf_df_result = if let Some(pre_tokenized) = &params.pre_tokenized {
        // Use pre-tokenized content if available
        if debug_mode {
            println!("DEBUG: Using pre-tokenized content for ranking");
        }
        compute_tf_df_from_tokenized(pre_tokenized, &query_token_map)
    } else {
        // Fallback to tokenizing the documents
        if debug_mode {
            println!("DEBUG: Tokenizing documents for ranking");
        }
        // Tokenize documents on the fly
        let tokenized_docs: Vec<Vec<String>> =
            params.documents.iter().map(|doc| tokenize(doc)).collect();
        compute_tf_df_from_tokenized(&tokenized_docs, &query_token_map)
    };

    let n_docs = params.documents.len();
    let avgdl = compute_avgdl(&tf_df_result.document_lengths);

    // Precompute IDF values

    let precomputed_idfs =
        precompute_idfs(&query_terms, &tf_df_result.document_frequencies, n_docs);

    if debug_mode {
        println!(
            "DEBUG: Precomputed IDF values for {} unique query terms",
            precomputed_idfs.len()
        );
    }

    // 4) BM25 parameters
    // These values are standard defaults for BM25 as established in academic literature:
    // k1=1.2 controls term frequency saturation (higher values give more weight to term frequency)
    // b=0.75 controls document length normalization (higher values give more penalty to longer documents)
    // See: Robertson, S. E., & Zaragoza, H. (2009). The Probabilistic Relevance Framework: BM25 and Beyond
    let k1 = 1.5; // EXPERIMENT: Slightly increased from 1.2 for balanced term frequency weight
    let b = 0.5; // EXPERIMENT: Moderately reduced from 0.75 for balanced length normalization

    if debug_mode {
        println!("DEBUG: Starting parallel document scoring for {n_docs} documents");
    }

    // 5) Compute BM25 bool logic score for each doc in parallel
    // Use a stable collection method to ensure deterministic ordering
    let scored_docs: Vec<(usize, Option<f64>)> = (0..tf_df_result.term_frequencies.len())
        .collect::<Vec<_>>() // Collect indices first to ensure stable ordering
        .par_iter() // Then parallelize
        .map(|&i| {
            let doc_tf = &tf_df_result.term_frequencies[i];
            let doc_len = tf_df_result.document_lengths[i];

            // Create optimized BM25 parameters with precomputed IDF values
            let precomputed_bm25_params = PrecomputedBm25Params {
                doc_tf,
                doc_len,
                avgdl,
                idfs: &precomputed_idfs,
                query_token_map: &query_token_map,
                k1,
                b,
            };

            // Evaluate doc's BM25 sum or None if excluded using optimized function
            let bm25_score_opt = score_expr_bm25_optimized(&parsed_expr, &precomputed_bm25_params);

            (i, bm25_score_opt)
        })
        .collect();

    if debug_mode {
        println!("DEBUG: Parallel document scoring completed");
    }

    // Filter out documents that didn't match and collect scores
    let mut filtered_docs: Vec<(usize, f64)> = scored_docs
        .into_iter()
        .filter_map(|(i, score_opt)| score_opt.map(|score| (i, score)))
        .collect();

    // 6) Sort in descending order by BM25 score, with a stable secondary sort by document index
    filtered_docs.sort_by(|a, b| {
        // First compare by score (descending)
        // Note: unwrap_or(Ordering::Equal) handles NaN cases by treating them as equal
        // This ensures stable sorting even if a score calculation resulted in NaN
        // (which shouldn't happen with our implementation, but provides robustness)
        match b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal) {
            Ordering::Equal => {
                // If scores are equal, sort by document index (ascending) for stability
                a.0.cmp(&b.0)
            }
            other => other,
        }
    });

    if debug_mode {
        println!(
            "DEBUG: Sorted {} matching documents by score",
            filtered_docs.len()
        );
    }

    filtered_docs
}

/// SIMD-optimized version of rank_documents using SimSIMD for vector operations
/// This provides significant performance improvements for large document sets
pub fn rank_documents_simd(params: &RankingParams) -> Vec<(usize, f64)> {
    use std::cmp::Ordering;

    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("DEBUG: Using SIMD-optimized ranking");
    }

    // 1) Parse the user query into an AST (Expr)
    let parsed_expr = match crate::search::elastic_query::parse_query(params.query, false) {
        Ok(expr) => expr,
        Err(e) => {
            if debug_mode {
                eprintln!("DEBUG: parse_query failed: {e:?}");
            }
            eprintln!("WARNING: Query parsing failed: {e:?}. Returning empty results.");
            return vec![];
        }
    };

    // 2) Extract query terms, create token mapping
    let query_terms = extract_query_terms(&parsed_expr);

    // Generate query token map (maps each unique query term to a unique u8 index)
    let query_token_map = match generate_query_token_map(&query_terms) {
        Ok(map) => map,
        Err(e) => {
            if debug_mode {
                eprintln!("DEBUG: Failed to generate query token map: {e}");
            }
            eprintln!("WARNING: {e}");
            return vec![];
        }
    };

    if debug_mode {
        println!(
            "DEBUG: Generated query token map with {} entries for SIMD processing",
            query_token_map.len()
        );
    }

    // 3) Precompute TF/DF for docs
    let tf_df_result = if let Some(pre_tokenized) = &params.pre_tokenized {
        if debug_mode {
            println!("DEBUG: Using pre-tokenized content for SIMD ranking");
        }
        compute_tf_df_from_tokenized(pre_tokenized, &query_token_map)
    } else {
        if debug_mode {
            println!("DEBUG: Tokenizing documents for SIMD ranking");
        }
        let tokenized_docs: Vec<Vec<String>> =
            params.documents.iter().map(|doc| tokenize(doc)).collect();
        compute_tf_df_from_tokenized(&tokenized_docs, &query_token_map)
    };

    let n_docs = params.documents.len();
    let avgdl = compute_avgdl(&tf_df_result.document_lengths);

    // 4) Precompute IDF values
    let precomputed_idfs =
        precompute_idfs(&query_terms, &tf_df_result.document_frequencies, n_docs);

    if debug_mode {
        println!(
            "DEBUG: Precomputed IDF values for {} unique query terms for SIMD processing",
            precomputed_idfs.len()
        );
    }

    // 5) Create sparse document matrix for SIMD operations
    let sparse_matrix = SparseDocumentMatrix::from_ranking_data(
        &tf_df_result.term_frequencies,
        &tf_df_result.document_lengths,
        &query_terms.iter().cloned().collect::<Vec<_>>(),
        &query_token_map,
        &precomputed_idfs,
        avgdl,
    );

    if debug_mode {
        println!(
            "DEBUG: Created sparse matrix with {} documents for SIMD operations",
            sparse_matrix.documents.len()
        );
    }

    // 6) Create SIMD BM25 parameters
    let simd_params = SimdBm25Params::new(&sparse_matrix, &tf_df_result.document_lengths);

    // 7) Compute BM25 scores using SIMD operations
    if debug_mode {
        println!("DEBUG: Starting SIMD-accelerated BM25 scoring for {n_docs} documents");
    }

    let scores = simd_params.score_all_documents();

    if debug_mode {
        println!("DEBUG: SIMD BM25 scoring completed");
    }

    // 8) Apply boolean query logic efficiently with SIMD scores
    // Only do traditional computation for documents that have SIMD scores > 0
    let scored_docs: Vec<(usize, f64)> = scores
        .into_iter()
        .enumerate()
        .filter_map(|(i, simd_score)| {
            if simd_score <= 0.0 {
                return None; // Skip documents with no matching terms
            }

            let doc_tf = &tf_df_result.term_frequencies[i];
            let doc_len = tf_df_result.document_lengths[i];

            // Create traditional BM25 parameters for boolean expression evaluation
            let precomputed_bm25_params = PrecomputedBm25Params {
                doc_tf,
                doc_len,
                avgdl,
                idfs: &precomputed_idfs,
                query_token_map: &query_token_map,
                k1: 1.5, // EXPERIMENT: Slightly increased from 1.2
                b: 0.5,  // EXPERIMENT: Moderately reduced from 0.75
            };

            // Apply boolean query logic - this filters out documents that don't match requirements
            score_expr_bm25_optimized(&parsed_expr, &precomputed_bm25_params)
                .map(|expr_score| (i, expr_score))
        })
        .collect();

    if debug_mode {
        println!("DEBUG: SIMD-filtered boolean query evaluation completed");
    }

    // 9) Sort results by SIMD score in descending order
    let mut filtered_docs = scored_docs;
    filtered_docs.sort_by(
        |a, b| match b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal) {
            Ordering::Equal => a.0.cmp(&b.0),
            other => other,
        },
    );

    if debug_mode {
        println!(
            "DEBUG: SIMD-optimized ranking completed with {} matching documents",
            filtered_docs.len()
        );
    }

    filtered_docs
}

/// Simple SIMD-only BM25 ranking for basic queries (no complex boolean logic)
/// This provides maximum performance for straightforward term-based queries
pub fn rank_documents_simd_simple(params: &RankingParams) -> Vec<(usize, f64)> {
    use std::cmp::Ordering;

    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("DEBUG: Using simple SIMD-only ranking (no boolean logic)");
    }

    // 1) Simple tokenization of query (treat as individual terms)
    let query_terms_vec = tokenize(params.query);
    let query_terms: HashSet<String> = query_terms_vec.into_iter().collect();

    if query_terms.is_empty() {
        return vec![];
    }

    // Generate query token map
    let query_token_map = match generate_query_token_map(&query_terms) {
        Ok(map) => map,
        Err(e) => {
            if debug_mode {
                eprintln!("DEBUG: Failed to generate query token map: {e}");
            }
            return vec![];
        }
    };

    // 2) Precompute TF/DF for docs
    let tf_df_result = if let Some(pre_tokenized) = &params.pre_tokenized {
        compute_tf_df_from_tokenized(pre_tokenized, &query_token_map)
    } else {
        let tokenized_docs: Vec<Vec<String>> =
            params.documents.iter().map(|doc| tokenize(doc)).collect();
        compute_tf_df_from_tokenized(&tokenized_docs, &query_token_map)
    };

    let n_docs = params.documents.len();
    let avgdl = compute_avgdl(&tf_df_result.document_lengths);

    // 3) Precompute IDF values
    let precomputed_idfs =
        precompute_idfs(&query_terms, &tf_df_result.document_frequencies, n_docs);

    // 4) Create sparse document matrix for pure SIMD operations
    let sparse_matrix = SparseDocumentMatrix::from_ranking_data(
        &tf_df_result.term_frequencies,
        &tf_df_result.document_lengths,
        &query_terms.iter().cloned().collect::<Vec<_>>(),
        &query_token_map,
        &precomputed_idfs,
        avgdl,
    );

    // 5) Compute pure SIMD BM25 scores
    let simd_params = SimdBm25Params::new(&sparse_matrix, &tf_df_result.document_lengths);
    let scores = simd_params.score_all_documents();

    if debug_mode {
        println!(
            "DEBUG: Pure SIMD scoring completed for {} documents",
            scores.len()
        );
    }

    // 6) Create results with scores > 0
    let mut filtered_docs: Vec<(usize, f64)> = scores
        .into_iter()
        .enumerate()
        .filter_map(|(i, score)| {
            if score > 0.0 {
                Some((i, score as f64))
            } else {
                None
            }
        })
        .collect();

    // 7) Sort by score (descending)
    filtered_docs.sort_by(
        |a, b| match b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal) {
            Ordering::Equal => a.0.cmp(&b.0),
            other => other,
        },
    );

    if debug_mode {
        println!(
            "DEBUG: Simple SIMD ranking completed with {} matching documents",
            filtered_docs.len()
        );
    }

    filtered_docs
}

/// Computes term frequencies (TF) for each document, document frequencies (DF) for each term,
/// and document lengths from pre-tokenized content.
///
/// Uses the query_token_map to convert string tokens to u8 indices for term frequencies.
pub fn compute_tf_df_from_tokenized(
    tokenized_docs: &[Vec<String>],
    query_token_map: &QueryTokenMap,
) -> TfDfResult {
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!("DEBUG: Starting parallel TF-DF computation from pre-tokenized content for {docs_len} documents", docs_len = tokenized_docs.len());
    }

    // Process documents in parallel to compute term frequencies and document lengths
    #[allow(clippy::type_complexity)]
    let doc_results: Vec<(
        HashMap<u8, usize>,
        HashMap<String, usize>,
        usize,
        HashSet<String>,
    )> = tokenized_docs
        .par_iter()
        .map(|tokens| {
            let mut tf_u8 = HashMap::new(); // Term frequencies using u8 indices
            let mut tf_str = HashMap::new(); // Term frequencies using strings (for DF calculation)

            // Compute term frequency for the current document
            for token in tokens.iter() {
                // Update string-based term frequency (for document frequency calculation)
                *tf_str.entry(token.clone()).or_insert(0) += 1;

                // Update u8-based term frequency (only for tokens in the query)
                if let Some(&token_index) = query_token_map.get(token) {
                    *tf_u8.entry(token_index).or_insert(0) += 1;
                }
            }

            // Collect unique terms for document frequency calculation
            let unique_terms: HashSet<String> = tf_str.keys().cloned().collect();

            (tf_u8, tf_str, tokens.len(), unique_terms)
        })
        .collect();

    // Extract term frequencies and document lengths
    let mut term_frequencies = Vec::with_capacity(tokenized_docs.len());
    let mut document_lengths = Vec::with_capacity(tokenized_docs.len());

    // Compute document frequencies in parallel using adaptive chunking
    // This balances parallelism with reduced contention
    // The chunk size calculation:
    // - Divides total documents by available threads to distribute work evenly
    // - Ensures at least 1 document per chunk to prevent empty chunks
    // - Larger chunks reduce thread coordination overhead but may lead to load imbalance
    // - Smaller chunks improve load balancing but increase synchronization costs
    // Use checked_div to safely handle the case where there are no threads (which shouldn't happen)
    // and ensure we always have at least one item per chunk
    let min_chunk_size = tokenized_docs
        .len()
        .checked_div(rayon::current_num_threads())
        .unwrap_or(1)
        .max(1);
    let document_frequencies = doc_results
        .par_iter()
        .with_min_len(min_chunk_size) // Adaptive chunking based on document count
        .map(|(_, _, _, unique_terms)| {
            // Create a local document frequency map for this chunk
            let mut local_df = HashMap::new();
            for term in unique_terms {
                *local_df.entry(term.clone()).or_insert(0) += 1;
            }
            local_df
        })
        .reduce(HashMap::new, |mut acc, local_df| {
            // Merge local document frequency maps
            for (term, count) in local_df {
                *acc.entry(term).or_insert(0) += count;
            }
            acc
        });

    if debug_mode {
        println!(
            "DEBUG: Parallel DF computation completed with {} unique terms",
            document_frequencies.len()
        );
    }

    // Collect results in a deterministic order
    for (tf_u8, _, doc_len, _) in doc_results {
        term_frequencies.push(tf_u8);
        document_lengths.push(doc_len);
    }

    if debug_mode {
        println!("DEBUG: Parallel TF-DF computation from pre-tokenized content completed");
        println!("DEBUG: Using u8 indices for term frequencies (optimized storage)");
    }

    TfDfResult {
        term_frequencies,
        document_frequencies,
        document_lengths,
    }
}

// -------------------------------------------------------------------------
// Unit tests (optional). Adapt or remove as you wish.
// -------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_bm25_scoring() {
        // A trivial test: 2 docs, 1 query
        let docs = vec!["api process load", "another random text with process"];
        let query = "+api +process +load"; // must have "api", must have "process", must have "load"

        let params = RankingParams {
            documents: &docs,
            query,
            pre_tokenized: None,
        };

        let results = rank_documents(&params);
        // Only the first doc should match, because it has all 3 required words
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 0); // doc index 0

        // Verify score is positive and within expected range for BM25
        // BM25 scores typically fall within certain ranges based on the algorithm's properties
        assert!(results[0].1 > 0.0);
        assert!(results[0].1 < 10.0); // Upper bound based on typical BM25 behavior with small documents
    }

    #[test]
    fn test_bm25_scoring_with_pre_tokenized() {
        // A trivial test: 2 docs, 1 query, with pre-tokenized content
        let docs = vec!["api process load", "another random text with process"];
        let query = "+api +process +load"; // must have "api", must have "process", must have "load"

        // Pre-tokenized content
        let pre_tokenized = vec![
            vec!["api".to_string(), "process".to_string(), "load".to_string()],
            vec![
                "another".to_string(),
                "random".to_string(),
                "text".to_string(),
                "with".to_string(),
                "process".to_string(),
            ],
        ];

        let params = RankingParams {
            documents: &docs,
            query,
            pre_tokenized: Some(&pre_tokenized),
        };

        let results = rank_documents(&params);
        // Only the first doc should match, because it has all 3 required words
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 0); // doc index 0

        // Verify score is positive and within expected range for BM25
        assert!(results[0].1 > 0.0);
        assert!(results[0].1 < 10.0); // Upper bound based on typical BM25 behavior with small documents
    }

    #[test]
    fn test_relative_bm25_scoring() {
        // Test that documents with more matching terms get higher scores
        let docs = vec![
            "api process load data", // 4 matching terms
            "api process load",      // 3 matching terms
            "api process",           // 2 matching terms
            "api",                   // 1 matching term
        ];
        let query = "api process load data"; // All terms are optional

        let params = RankingParams {
            documents: &docs,
            query,
            pre_tokenized: None,
        };

        let results = rank_documents(&params);
        // All docs should match since all terms are optional
        assert_eq!(results.len(), 4);

        // Verify that scores decrease as fewer terms match
        // Doc with 4 matches should be first, then 3, then 2, then 1
        assert_eq!(results[0].0, 0); // First doc (4 matches)
        assert_eq!(results[1].0, 1); // Second doc (3 matches)
        assert_eq!(results[2].0, 2); // Third doc (2 matches)
        assert_eq!(results[3].0, 3); // Fourth doc (1 match)

        // Verify that scores decrease as expected
        assert!(results[0].1 > results[1].1); // 4 matches > 3 matches
        assert!(results[1].1 > results[2].1); // 3 matches > 2 matches
        assert!(results[2].1 > results[3].1); // 2 matches > 1 match
    }

    #[test]
    fn test_generate_query_token_map_basic() {
        // Create a set of query terms
        let mut query_terms = HashSet::new();
        query_terms.insert("apple".to_string());
        query_terms.insert("banana".to_string());
        query_terms.insert("cherry".to_string());

        // Generate the token map
        let token_map = generate_query_token_map(&query_terms).unwrap();

        // Verify the map contains all terms
        assert_eq!(token_map.len(), 3);

        // Verify each term has a unique index
        let mut indices = HashSet::new();
        for (_, &idx) in &token_map {
            assert!(indices.insert(idx), "Duplicate index found");
        }

        // Verify the indices are in the expected range
        assert_eq!(indices.len(), 3);
        assert!(indices.contains(&0));
        assert!(indices.contains(&1));
        assert!(indices.contains(&2));
    }

    #[test]
    fn test_generate_query_token_map_empty() {
        // Test with empty query terms
        let query_terms = HashSet::new();

        // Generate the token map
        let token_map = generate_query_token_map(&query_terms).unwrap();

        // Verify the map is empty
        assert!(token_map.is_empty());
    }

    #[test]
    fn test_generate_query_token_map_deterministic() {
        // Create two identical sets of query terms
        let mut query_terms1 = HashSet::new();
        query_terms1.insert("apple".to_string());
        query_terms1.insert("banana".to_string());
        query_terms1.insert("cherry".to_string());

        let mut query_terms2 = HashSet::new();
        query_terms2.insert("cherry".to_string());
        query_terms2.insert("apple".to_string());
        query_terms2.insert("banana".to_string());

        // Generate token maps for both sets
        let token_map1 = generate_query_token_map(&query_terms1).unwrap();
        let token_map2 = generate_query_token_map(&query_terms2).unwrap();

        // Verify both maps have the same mappings despite different insertion order
        assert_eq!(token_map1.len(), token_map2.len());

        for (term, &idx1) in &token_map1 {
            assert_eq!(
                Some(&idx1),
                token_map2.get(term),
                "Term '{term}' has different indices in the two maps"
            );
        }
    }

    #[test]
    fn test_generate_query_token_map_too_many_terms() {
        // Create a set with more than 256 terms
        let query_terms: HashSet<String> = (0..257).map(|i| format!("term{i}")).collect();

        // Attempt to generate the token map
        let result = generate_query_token_map(&query_terms);

        // Verify it returns an error
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Query exceeds the 256 unique token limit for u8 mapping"
        );
    }

    #[test]
    fn test_compute_tf_df_with_u8_indices() {
        // Create a simple set of documents
        let docs = vec![
            vec![
                "apple".to_string(),
                "banana".to_string(),
                "cherry".to_string(),
            ],
            vec!["apple".to_string(), "banana".to_string()],
            vec!["apple".to_string()],
        ];

        // Create a query token map
        let mut query_token_map = QueryTokenMap::new();
        query_token_map.insert("apple".to_string(), 0);
        query_token_map.insert("banana".to_string(), 1);
        query_token_map.insert("cherry".to_string(), 2);

        // Compute TF/DF
        let tf_df_result = compute_tf_df_from_tokenized(&docs, &query_token_map);

        // Check document lengths
        assert_eq!(tf_df_result.document_lengths[0], 3);
        assert_eq!(tf_df_result.document_lengths[1], 2);
        assert_eq!(tf_df_result.document_lengths[2], 1);

        // Check term frequencies using u8 indices
        assert_eq!(*tf_df_result.term_frequencies[0].get(&0).unwrap(), 1); // "apple" in doc 0
        assert_eq!(*tf_df_result.term_frequencies[0].get(&1).unwrap(), 1); // "banana" in doc 0
        assert_eq!(*tf_df_result.term_frequencies[0].get(&2).unwrap(), 1); // "cherry" in doc 0

        assert_eq!(*tf_df_result.term_frequencies[1].get(&0).unwrap(), 1); // "apple" in doc 1
        assert_eq!(*tf_df_result.term_frequencies[1].get(&1).unwrap(), 1); // "banana" in doc 1
        assert!(tf_df_result.term_frequencies[1].get(&2).is_none()); // no "cherry" in doc 1

        assert_eq!(*tf_df_result.term_frequencies[2].get(&0).unwrap(), 1); // "apple" in doc 2
        assert!(tf_df_result.term_frequencies[2].get(&1).is_none()); // no "banana" in doc 2
        assert!(tf_df_result.term_frequencies[2].get(&2).is_none()); // no "cherry" in doc 2

        // Check document frequencies (still string-based)
        assert_eq!(*tf_df_result.document_frequencies.get("apple").unwrap(), 3); // in all 3 docs
        assert_eq!(*tf_df_result.document_frequencies.get("banana").unwrap(), 2); // in 2 docs
        assert_eq!(*tf_df_result.document_frequencies.get("cherry").unwrap(), 1);
        // in 1 doc
    }

    #[test]
    fn test_bm25_scoring_with_u8_indices() {
        // Create a simple document
        let _doc_content = "apple banana cherry"; // Kept for documentation purposes

        // Create a query token map
        let mut query_token_map = QueryTokenMap::new();
        query_token_map.insert("apple".to_string(), 0);
        query_token_map.insert("banana".to_string(), 1);
        query_token_map.insert("cherry".to_string(), 2);

        // Create term frequencies using u8 indices
        let mut doc_tf = HashMap::new();
        doc_tf.insert(0u8, 1); // "apple" appears once
        doc_tf.insert(1u8, 1); // "banana" appears once
        doc_tf.insert(2u8, 1); // "cherry" appears once

        // Create document frequencies
        let mut doc_freqs = HashMap::new();
        doc_freqs.insert("apple".to_string(), 1);
        doc_freqs.insert("banana".to_string(), 1);
        doc_freqs.insert("cherry".to_string(), 1);

        // Create IDF values (simplified for testing)
        let mut idfs = HashMap::new();
        idfs.insert("apple".to_string(), 1.0);
        idfs.insert("banana".to_string(), 1.0);
        idfs.insert("cherry".to_string(), 1.0);

        // Create BM25 parameters
        let params = PrecomputedBm25Params {
            doc_tf: &doc_tf,
            doc_len: 3,
            avgdl: 3.0,
            idfs: &idfs,
            query_token_map: &query_token_map,
            k1: 1.2,
            b: 0.75,
        };

        // Test bm25_single_token_optimized
        let apple_score = bm25_single_token_optimized("apple", &params);
        let banana_score = bm25_single_token_optimized("banana", &params);
        let cherry_score = bm25_single_token_optimized("cherry", &params);

        // All terms have the same frequency, IDF, and document length,
        // so they should have the same score
        assert!(apple_score > 0.0);
        assert_eq!(apple_score, banana_score);
        assert_eq!(banana_score, cherry_score);

        // Test with a term not in the query_token_map
        let unknown_score = bm25_single_token_optimized("unknown", &params);
        assert_eq!(unknown_score, 0.0);

        // Test score_term_bm25_optimized
        let keywords = vec!["apple".to_string(), "banana".to_string()];
        let term_score = score_term_bm25_optimized(&keywords, &params);

        // The score should be the sum of individual scores
        assert_eq!(term_score, apple_score + banana_score);
    }
}
