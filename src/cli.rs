use clap::{Parser as ClapParser, Subcommand};
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(author, version, about = "AI-friendly, fully local, semantic code search tool for large codebases", long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Search code using patterns
    Search {
        /// Search pattern
        #[arg(value_name = "PATTERN")]
        pattern: String,

        /// Files or directories to search
        #[arg(value_name = "PATH", default_value = ".")]
        paths: Vec<PathBuf>,

        /// Skip AST parsing and just output unique files
        #[arg(short, long = "files-only")]
        files_only: bool,

        /// Custom patterns to ignore (in addition to .gitignore and common patterns)
        #[arg(short, long)]
        ignore: Vec<String>,

        /// Include files whose names match query words
        #[arg(short = 'n', long = "include-filenames")]
        include_filenames: bool,

        /// Reranking method to use for search results
        #[arg(short = 'r', long = "reranker", default_value = "hybrid", value_parser = ["hybrid", "hybrid2", "bm25", "tfidf"])]
        reranker: String,

        /// Use frequency-based search with stemming and stopword removal (enabled by default)
        #[arg(short = 's', long = "frequency", default_value = "true")]
        frequency_search: bool,

        /// Use exact matching without stemming or stopword removal
        #[arg(long = "exact")]
        exact: bool,

        /// Maximum number of results to return
        #[arg(long = "max-results")]
        max_results: Option<usize>,

        /// Maximum total bytes of code content to return
        #[arg(long = "max-bytes")]
        max_bytes: Option<usize>,

        /// Maximum total tokens in code content to return (for AI usage)
        #[arg(long = "max-tokens")]
        max_tokens: Option<usize>,

        /// Allow test files and test code blocks in search results
        #[arg(long = "allow-tests")]
        allow_tests: bool,

        /// Match files that contain any of the search terms (by default, files must contain all terms)
        #[arg(long = "any-term")]
        any_term: bool,

        /// Disable merging of adjacent code blocks after ranking (merging enabled by default)
        #[arg(long = "no-merge", default_value = "false")]
        no_merge: bool,

        /// Maximum number of lines between code blocks to consider them adjacent for merging (default: 5)
        #[arg(long = "merge-threshold")]
        merge_threshold: Option<usize>,
    },

    /// Use AI chat to interact with codebase
    Chat,
}
