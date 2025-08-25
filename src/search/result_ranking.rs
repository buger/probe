#[cfg(feature = "bert-reranker")]
use probe_code::bert_reranker;
use probe_code::models::SearchResult;
use probe_code::ranking;
use std::time::Instant;

/// Helper function to format duration in a human-readable way
fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_millis() < 1000 {
        let millis = duration.as_millis();
        format!("{millis}ms")
    } else {
        let secs = duration.as_secs_f64();
        format!("{secs:.2}s")
    }
}

/// Function to rank search results based on query relevance using various algorithms
pub fn rank_search_results(
    results: &mut [SearchResult],
    queries: &[String],
    reranker: &str,
    question: Option<&str>,
) {
    let start_time = Instant::now();

    // Check if debug mode is enabled
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    if debug_mode {
        println!(
            "DEBUG: Starting result ranking with {} results",
            results.len()
        );
        println!("DEBUG: Using reranker: {reranker}");
        println!("DEBUG: Queries: {queries:?}");
    }

    // Handle BERT-based reranking for MS-MARCO models
    if reranker == "ms-marco-tinybert"
        || reranker == "ms-marco-minilm-l6"
        || reranker == "ms-marco-minilm-l12"
    {
        handle_bert_reranking(results, queries, reranker, question, debug_mode, start_time);
        return;
    }

    // Combine all queries into a single string for ranking
    let query_combine_start = Instant::now();
    let combined_query = queries.join(" ");
    let query_combine_duration = query_combine_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Query combination completed in {} - Combined query: '{}'",
            format_duration(query_combine_duration),
            combined_query
        );
    }

    // Extract document texts for ranking, including filename in each document
    let document_extraction_start = Instant::now();
    // This ensures filename terms are considered in the ranking algorithms
    let documents: Vec<String> = results
        .iter()
        .map(|r| {
            let mut doc = String::with_capacity(r.file.len() + r.code.len() + 15);
            doc.push_str("// Filename: ");
            doc.push_str(&r.file);
            doc.push('\n');
            doc.push_str(&r.code);
            doc
        })
        .collect();
    let documents_refs: Vec<&str> = documents.iter().map(|s| s.as_str()).collect();
    let document_extraction_duration = document_extraction_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Document extraction completed in {} - Extracted {} documents",
            format_duration(document_extraction_duration),
            documents.len()
        );
    }

    // Rank the documents
    let metrics_extraction_start = Instant::now();
    // Get metrics from the first result (assuming they're the same for all results in this set)
    let file_unique_terms = results.first().and_then(|r| r.file_unique_terms);
    let file_total_matches = results.first().and_then(|r| r.file_total_matches);
    let block_unique_terms = results.first().and_then(|r| r.block_unique_terms);
    let block_total_matches = results.first().and_then(|r| r.block_total_matches);
    let metrics_extraction_duration = metrics_extraction_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Metrics extraction completed in {}",
            format_duration(metrics_extraction_duration)
        );
        println!(
            "DEBUG: Extracted metrics - file_unique_terms: {file_unique_terms:?}, file_total_matches: {file_total_matches:?}, block_unique_terms: {block_unique_terms:?}, block_total_matches: {block_total_matches:?}"
        );
    }

    // Extract pre-tokenized content if available
    let tokenized_extraction_start = Instant::now();
    let pre_tokenized: Vec<Vec<String>> = results
        .iter()
        .filter_map(|r| r.tokenized_content.clone())
        .collect();

    let has_tokenized = !pre_tokenized.is_empty() && pre_tokenized.len() == results.len();

    if debug_mode {
        if has_tokenized {
            println!(
                "DEBUG: Using pre-tokenized content from {} results",
                pre_tokenized.len()
            );
        } else {
            println!(
                "DEBUG: Pre-tokenized content not available for all results (found {}/{}), falling back to tokenization",
                pre_tokenized.len(),
                results.len()
            );
        }
    }

    let tokenized_extraction_duration = tokenized_extraction_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Tokenized content extraction completed in {}",
            format_duration(tokenized_extraction_duration)
        );
    }

    let ranking_params = ranking::RankingParams {
        documents: &documents_refs,
        query: &combined_query,
        pre_tokenized: if has_tokenized {
            Some(&pre_tokenized)
        } else {
            None
        },
    };

    let document_ranking_start = Instant::now();
    if debug_mode {
        println!("DEBUG: Starting document ranking...");
    }

    // Get ranked indices from the ranking module (BM25 scores)
    // Use SIMD-optimized ranking by default, can be disabled with DISABLE_SIMD_RANKING=1
    let use_simd = std::env::var("DISABLE_SIMD_RANKING").unwrap_or_default() != "1";
    let ranked_indices = if use_simd {
        if debug_mode {
            println!("DEBUG: Using SIMD-optimized ranking (default)");
        }
        ranking::rank_documents_simd(&ranking_params)
    } else {
        if debug_mode {
            println!("DEBUG: Using traditional ranking (SIMD disabled)");
        }
        ranking::rank_documents(&ranking_params)
    };

    let document_ranking_duration = document_ranking_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Document ranking completed in {} - Ranked {} documents",
            format_duration(document_ranking_duration),
            ranked_indices.len()
        );
    }

    // Update scores for all results returned by the ranking module
    // We don't filter by BM25 score here because the ranking module already does some filtering
    // based on the query, and we want to preserve OR query behavior
    let filtering_start = Instant::now();
    let mut updated_results = Vec::new();

    // Update scores for all results
    for (rank_index, (original_index, bm25_score)) in ranked_indices.iter().enumerate() {
        if let Some(result) = results.get(*original_index) {
            let mut result_clone = result.clone();
            result_clone.rank = Some(rank_index + 1); // 1-based rank

            // EXPERIMENT: Apply node type boosting for better relevance
            let node_type_boost = match result_clone.node_type.as_str() {
                // Function/method implementations are most relevant (2.0x boost)
                "function_item"
                | "function_declaration"
                | "method_declaration"
                | "function_definition"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "method"
                | "singleton_method"
                | "constructor_declaration" => 2.0,

                // Type definitions and implementations are highly relevant (1.8x boost)
                "impl_item"
                | "struct_item"
                | "class_declaration"
                | "type_definition"
                | "interface_declaration"
                | "class_specifier"
                | "struct_specifier"
                | "struct_declaration"
                | "interface_type"
                | "protocol_declaration"
                | "type_alias_declaration"
                | "typealias_declaration" => 1.8,

                // Enums, traits, and type specifications (1.6x boost)
                "enum_item"
                | "trait_item"
                | "enum_declaration"
                | "enum_specifier"
                | "type_declaration"
                | "type_spec"
                | "trait_declaration"
                | "extension_declaration"
                | "delegate_declaration" => 1.6,

                // Module, namespace, and package definitions (1.4x boost)
                "module"
                | "mod_item"
                | "namespace"
                | "namespace_declaration"
                | "namespace_definition"
                | "module_declaration"
                | "package_declaration" => 1.4,

                // Properties, constants, and event declarations (1.3x boost)
                "property_declaration"
                | "event_declaration"
                | "const_declaration"
                | "var_declaration"
                | "variable_declaration"
                | "constant_declaration"
                | "const_spec"
                | "var_spec" => 1.3,

                // Documentation blocks for functions (multi-line) (1.2x boost)
                "doc_comment" | "block_comment"
                    if result_clone.lines.1 - result_clone.lines.0 > 3 =>
                {
                    1.2
                }

                // Export statements and declarations (1.1x boost)
                "export_statement" | "declare_statement" | "declaration" => 1.1,

                // Test code is less relevant (0.7x penalty)
                node_type if node_type.contains("test") || node_type.contains("Test") => 0.7,

                // Single line comments are least relevant (0.5x penalty)
                "line_comment" | "comment" | "//" | "/*" | "*/" => 0.5,

                // Other acceptable but less specific node types (1.0x - no change)
                "object"
                | "array"
                | "jsx_element"
                | "jsx_self_closing_element"
                | "property_identifier"
                | "class_body"
                | "class"
                | "identifier" => 1.0,

                // Default for any other node types
                _ => 1.0,
            };

            let boosted_score = bm25_score * node_type_boost;
            result_clone.score = Some(boosted_score);
            result_clone.bm25_score = Some(*bm25_score); // Keep original BM25 score
            updated_results.push(result_clone);
        }
    }

    let updated_len = updated_results.len();

    if debug_mode {
        println!("DEBUG: Score update completed - Updated {updated_len} results");
    }

    // Sort updated results by BM25 score in descending order
    let reranker_sort_start = Instant::now();

    if debug_mode {
        println!("DEBUG: Using BM25 ranking (Okapi BM25 algorithm)");
    } else {
        println!("Using BM25 ranking (Okapi BM25 algorithm)");
    }

    // Sort by boosted score in descending order
    updated_results.sort_by(|a, b| {
        let score_a = a.score.unwrap_or(0.0); // Use boosted score
        let score_b = b.score.unwrap_or(0.0); // Use boosted score
                                              // Sort in descending order (higher score is better)
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Reassign ranks based on the sorted order
    for (rank, result) in updated_results.iter_mut().enumerate() {
        result.bm25_rank = Some(rank + 1); // 1-based rank
        result.rank = Some(rank + 1);
    }

    let reranker_sort_duration = reranker_sort_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Reranker-specific sorting completed in {}",
            format_duration(reranker_sort_duration)
        );
    }

    // Replace original results with updated results
    if updated_len < results.len() {
        // If we have fewer results than the original array, copy only what we have
        for (i, result) in updated_results.into_iter().enumerate() {
            results[i] = result;
        }

        // Instead of truncating, we'll keep the original file paths
        // but mark these results with a special flag
        for result in results.iter_mut().skip(updated_len) {
            // Set a special flag to indicate this result should be skipped
            // but preserve the file path
            result.matched_by_filename = Some(false);
            result.score = Some(0.0);
            result.rank = Some(usize::MAX);
        }
    } else {
        // If we have the same number of results, just replace them all
        for (i, result) in updated_results.into_iter().enumerate() {
            results[i] = result;
        }
    }

    let filtering_duration = filtering_start.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Result processing completed in {} - Processed {} results",
            format_duration(filtering_duration),
            updated_len
        );
    }

    let total_duration = start_time.elapsed();

    if debug_mode {
        println!(
            "DEBUG: Total result ranking completed in {}",
            format_duration(total_duration)
        );
    }
}

/// Handle BERT-based reranking using the ms-marco-tinybert model
fn handle_bert_reranking(
    results: &mut [SearchResult],
    queries: &[String],
    reranker: &str,
    question: Option<&str>,
    debug_mode: bool,
    start_time: Instant,
) {
    if debug_mode {
        println!("DEBUG: Using BERT reranking with {reranker}");
        if let Some(q) = question {
            println!("DEBUG: Using custom question for reranking: '{q}'");
        } else {
            println!("DEBUG: Using search keywords for reranking: {queries:?}");
        }
    } else {
        println!("Using BERT reranking with {reranker}");
        if let Some(q) = question {
            println!("Using custom question: '{q}'");
        }
    }

    #[cfg(feature = "bert-reranker")]
    {
        use tokio::runtime::Runtime;

        // Use thread-based approach to avoid nested runtime issues
        let bert_result = std::thread::spawn({
            let results_clone = results.to_vec();
            let queries_clone = queries.to_vec();
            let question_clone = question.map(|s| s.to_string());
            let reranker_clone = reranker.to_string();
            move || {
                let rt = Runtime::new().expect("Failed to create runtime for BERT reranking");
                rt.block_on(async {
                    // Create a mutable copy to work with
                    let mut results_copy = results_clone;

                    // Map reranker name to model name
                    let model_name = match reranker_clone.as_str() {
                        "ms-marco-tinybert" => "cross-encoder/ms-marco-TinyBERT-L-2-v2",
                        "ms-marco-minilm-l6" => "cross-encoder/ms-marco-MiniLM-L-6-v2",
                        "ms-marco-minilm-l12" => "cross-encoder/ms-marco-MiniLM-L-12-v2",
                        _ => "cross-encoder/ms-marco-TinyBERT-L-2-v2", // default fallback
                    };

                    bert_reranker::rerank_with_bert(
                        &mut results_copy,
                        &queries_clone,
                        model_name,
                        question_clone.as_deref(),
                    )
                    .await
                    .map(|_| results_copy)
                })
            }
        })
        .join();

        let bert_result = match bert_result {
            Ok(inner_result) => inner_result,
            Err(_) => {
                eprintln!("BERT reranking thread panicked");
                println!("Falling back to BM25 ranking...");
                fallback_to_bm25_ranking(results, queries, debug_mode, start_time);
                return;
            }
        };

        match bert_result {
            Ok(reranked_results) => {
                // Copy the reranked results back to the original slice
                for (i, reranked_result) in reranked_results.into_iter().enumerate() {
                    if i < results.len() {
                        results[i] = reranked_result;
                    }
                }

                let total_duration = start_time.elapsed();
                if debug_mode {
                    println!(
                        "DEBUG: BERT reranking completed successfully in {}",
                        format_duration(total_duration)
                    );
                }
            }
            Err(e) => {
                eprintln!("BERT reranking failed: {e}");
                println!("Falling back to BM25 ranking...");
                fallback_to_bm25_ranking(results, queries, debug_mode, start_time);
            }
        }
    }

    #[cfg(not(feature = "bert-reranker"))]
    {
        eprintln!("BERT reranker '{reranker}' is not available.");
        eprintln!("To enable BERT reranking, build with: cargo build --features bert-reranker");
        println!("Falling back to BM25 ranking...");
        fallback_to_bm25_ranking(results, queries, debug_mode, start_time);
    }
}

/// Fallback to BM25 ranking when BERT reranking fails or is unavailable
fn fallback_to_bm25_ranking(
    results: &mut [SearchResult],
    queries: &[String],
    debug_mode: bool,
    start_time: Instant,
) {
    // Reimplement the BM25 ranking logic that was in the main function
    let combined_query = queries.join(" ");

    // Extract document texts for ranking, including filename in each document
    let documents: Vec<String> = results
        .iter()
        .map(|r| {
            let mut doc = String::with_capacity(r.file.len() + r.code.len() + 15);
            doc.push_str("// Filename: ");
            doc.push_str(&r.file);
            doc.push('\n');
            doc.push_str(&r.code);
            doc
        })
        .collect();
    let documents_refs: Vec<&str> = documents.iter().map(|s| s.as_str()).collect();

    // Extract pre-tokenized content if available
    let pre_tokenized: Vec<Vec<String>> = results
        .iter()
        .filter_map(|r| r.tokenized_content.clone())
        .collect();

    let has_tokenized = !pre_tokenized.is_empty() && pre_tokenized.len() == results.len();

    let ranking_params = ranking::RankingParams {
        documents: &documents_refs,
        query: &combined_query,
        pre_tokenized: if has_tokenized {
            Some(&pre_tokenized)
        } else {
            None
        },
    };

    // Use SIMD-optimized ranking by default, can be disabled with DISABLE_SIMD_RANKING=1
    let use_simd = std::env::var("DISABLE_SIMD_RANKING").unwrap_or_default() != "1";
    let ranked_indices = if use_simd {
        if debug_mode {
            println!("DEBUG: Using SIMD-optimized BM25 ranking (fallback)");
        }
        ranking::rank_documents_simd(&ranking_params)
    } else {
        if debug_mode {
            println!("DEBUG: Using traditional BM25 ranking (fallback)");
        }
        ranking::rank_documents(&ranking_params)
    };

    // Update scores for all results
    let mut updated_results = Vec::new();

    for (rank_index, (original_index, bm25_score)) in ranked_indices.iter().enumerate() {
        if let Some(result) = results.get(*original_index) {
            let mut result_clone = result.clone();
            result_clone.rank = Some(rank_index + 1); // 1-based rank

            // Apply node type boosting (same logic as in the main function)
            let node_type_boost = match result_clone.node_type.as_str() {
                "function_item"
                | "function_declaration"
                | "method_declaration"
                | "function_definition"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
                | "method"
                | "singleton_method"
                | "constructor_declaration" => 2.0,
                "impl_item"
                | "struct_item"
                | "class_declaration"
                | "type_definition"
                | "interface_declaration"
                | "class_specifier"
                | "struct_specifier"
                | "struct_declaration"
                | "interface_type"
                | "protocol_declaration"
                | "type_alias_declaration"
                | "typealias_declaration" => 1.8,
                "enum_item"
                | "trait_item"
                | "enum_declaration"
                | "enum_specifier"
                | "type_declaration"
                | "type_spec"
                | "trait_declaration"
                | "extension_declaration"
                | "delegate_declaration" => 1.6,
                "module"
                | "mod_item"
                | "namespace"
                | "namespace_declaration"
                | "namespace_definition"
                | "module_declaration"
                | "package_declaration" => 1.4,
                "property_declaration"
                | "event_declaration"
                | "const_declaration"
                | "var_declaration"
                | "variable_declaration"
                | "constant_declaration"
                | "const_spec"
                | "var_spec" => 1.3,
                "doc_comment" | "block_comment"
                    if result_clone.lines.1 - result_clone.lines.0 > 3 =>
                {
                    1.2
                }
                "export_statement" | "declare_statement" | "declaration" => 1.1,
                node_type if node_type.contains("test") || node_type.contains("Test") => 0.7,
                "line_comment" | "comment" | "//" | "/*" | "*/" => 0.5,
                _ => 1.0,
            };

            let boosted_score = bm25_score * node_type_boost;
            result_clone.score = Some(boosted_score);
            result_clone.bm25_score = Some(*bm25_score);
            updated_results.push(result_clone);
        }
    }

    // Sort by boosted score in descending order
    updated_results.sort_by(|a, b| {
        let score_a = a.score.unwrap_or(0.0);
        let score_b = b.score.unwrap_or(0.0);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Reassign ranks based on the sorted order
    for (rank, result) in updated_results.iter_mut().enumerate() {
        result.bm25_rank = Some(rank + 1);
        result.rank = Some(rank + 1);
    }

    // Replace original results with updated results
    let updated_len = updated_results.len();
    if updated_len < results.len() {
        for (i, result) in updated_results.into_iter().enumerate() {
            results[i] = result;
        }
        // Mark remaining results as low priority
        for result in results.iter_mut().skip(updated_len) {
            result.matched_by_filename = Some(false);
            result.score = Some(0.0);
            result.rank = Some(usize::MAX);
        }
    } else {
        for (i, result) in updated_results.into_iter().enumerate() {
            results[i] = result;
        }
    }

    let total_duration = start_time.elapsed();
    if debug_mode {
        println!(
            "DEBUG: BM25 fallback ranking completed in {}",
            format_duration(total_duration)
        );
    }
}
