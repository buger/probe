// Structure to hold both limited search results and skipped files
#[derive(Debug)]
pub struct LimitedSearchResults {
    pub results: Vec<SearchResult>,
    pub skipped_files: Vec<SearchResult>,
    pub limits_applied: Option<SearchLimits>,
    pub cached_blocks_skipped: Option<usize>,
    pub files_skipped_early_termination: Option<usize>,
}

// Structure to track which limits were applied
#[derive(Debug)]
pub struct SearchLimits {
    pub max_results: Option<usize>,
    pub max_bytes: Option<usize>,
    pub max_tokens: Option<usize>,

    #[allow(dead_code)]
    pub total_bytes: usize,
    #[allow(dead_code)]
    pub total_tokens: usize,
}

// Structure to hold search results
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file: String,
    pub lines: (usize, usize),
    pub node_type: String,
    pub code: String,
    // Indicates if this result was found by filename matching
    pub matched_by_filename: Option<bool>,
    // Ranking information
    pub rank: Option<usize>,
    // Combined score from the ranking algorithm
    pub score: Option<f64>,
    // Individual TF-IDF score
    pub tfidf_score: Option<f64>,
    // Individual BM25 score
    pub bm25_score: Option<f64>,
    // TF-IDF rank (1 is most relevant)
    pub tfidf_rank: Option<usize>,
    // BM25 rank (1 is most relevant)
    pub bm25_rank: Option<usize>,
    // New score incorporating file and block metrics
    pub new_score: Option<f64>,
    // Hybrid2 rank (1 is most relevant)
    pub hybrid2_rank: Option<usize>,
    // Separate rank for combined score (useful when using different rerankers)
    pub combined_score_rank: Option<usize>,
    // Number of distinct search terms matched in the file
    pub file_unique_terms: Option<usize>,
    // Total count of matches across the file (content + filename matches)
    pub file_total_matches: Option<usize>,
    // Rank of the file based on total matches
    pub file_match_rank: Option<usize>,
    // Number of unique search terms matched in the block
    pub block_unique_terms: Option<usize>,
    // Total frequency of term matches in the block
    pub block_total_matches: Option<usize>,
    // Identifier for the parent file (used for block merging)
    pub parent_file_id: Option<String>,
    // Identifier for the individual block within a file (used for block merging)
    #[allow(dead_code)]
    pub block_id: Option<usize>,
    // The actual keywords that matched in this result
    pub matched_keywords: Option<Vec<String>>,
    /// Tokenized version of the code block with filename prepended
    #[allow(dead_code)]
    pub tokenized_content: Option<Vec<String>>,
    /// LSP-enhanced symbol information (call hierarchy, references, etc.)
    pub lsp_info: Option<serde_json::Value>,
}

// Structure to hold node information for merging
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CodeBlock {
    pub start_row: usize,
    pub end_row: usize,
    #[allow(dead_code)]
    pub start_byte: usize,
    #[allow(dead_code)]
    pub end_byte: usize,
    pub node_type: String,
    // Parent node information
    pub parent_node_type: Option<String>,
    pub parent_start_row: Option<usize>,
    pub parent_end_row: Option<usize>,
}
