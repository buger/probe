//! # Probe
//!
//! Probe is an AI-friendly, fully local, semantic code search tool for large codebases.
//!
//! This crate provides both a command-line interface and a library that can be used
//! programmatically in other Rust applications.
//!
//! ## Features
//!
//! - Semantic code search with intelligent ranking
//! - Code block extraction with language-aware parsing
//! - AST-based pattern matching for precise code structure search
//! - SIMD-accelerated vector operations for improved performance
//!
//! ## Examples
//!
//! ### Searching for code
//!
//! ```no_run
//! use probe_code::search::{perform_probe, SearchOptions};
//! use std::path::Path;
//!
//! // Create search options
//! let options = SearchOptions {
//!     path: Path::new("."),
//!     queries: &vec!["function search".to_string()],
//!     files_only: false,
//!     custom_ignores: &[],
//!     exclude_filenames: false,
//!     reranker: "bm25",
//!     frequency_search: true,
//!     exact: false,
//!     language: None,
//!     max_results: Some(10),
//!     max_bytes: None,
//!     max_tokens: Some(10000),
//!     allow_tests: false,
//!     no_merge: false,
//!     merge_threshold: None,
//!     dry_run: false,
//!     session: None,
//!     timeout: 30,
//! };
//!
//! let results = perform_probe(&options).unwrap();
//! println!("Found {} results", results.results.len());
//! ```
//!
//! ### Extracting code blocks
//!
//! ```no_run
//! use probe_code::extract::{handle_extract, ExtractOptions};
//!
//! let options = ExtractOptions {
//!     files: vec!["src/main.rs".to_string()],
//!     custom_ignores: vec![],
//!     context_lines: 0,
//!     format: "text".to_string(),
//!     from_clipboard: false,
//!     input_file: None,
//!     to_clipboard: false,
//!     dry_run: false,
//!     diff: false,
//!     allow_tests: false,
//!     keep_input: false,
//!     prompt: None,
//!     instructions: None,
//! };
//!
//! handle_extract(options).unwrap();
//! ```
//!
//! ### AST pattern matching
//!
//! ```no_run
//! use probe_code::query::{perform_query, QueryOptions};
//! use std::path::Path;
//!
//! // Using the lower-level perform_query function
//! let options = QueryOptions {
//!     path: Path::new("."),
//!     pattern: "fn $NAME($$$PARAMS) { $$$BODY }",
//!     language: Some("rust".to_string()),
//!     ignore: &[],
//!     allow_tests: false,
//!     max_results: None,
//!     format: "text".to_string(),
//! };
//!
//! let matches = perform_query(&options).unwrap();
//! println!("Found {} matches", matches.len());
//! ```

// Allow internal modules to reference the crate by its library name
extern crate self as probe_code;

pub mod bert_reranker;
pub mod config;
pub mod extract;
pub mod language;
pub mod lsp_integration;
pub mod models;
pub mod path_resolver;
pub mod path_safety;
pub mod query;
pub mod ranking;
pub mod search;
pub mod simd_ranking;
pub mod simd_test;
pub mod utils;
pub mod version;

// Re-export commonly used types for convenience
pub use extract::{
    format_and_print_extraction_results, handle_extract, process_file_for_extraction,
    ExtractOptions,
};
pub use models::{CodeBlock, LimitedSearchResults, SearchLimits, SearchResult};
pub use path_resolver::resolve_path;
pub use query::{
    format_and_print_query_results, handle_query, perform_query, AstMatch, QueryOptions,
};
pub use search::{format_and_print_search_results, perform_probe, SearchOptions};

// Tests are defined in their respective modules with #[cfg(test)]
