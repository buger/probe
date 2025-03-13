use std::path::Path;

/// Options for performing a search
pub struct SearchOptions<'a> {
    pub path: &'a Path,
    pub queries: &'a [String],
    pub files_only: bool,
    pub custom_ignores: &'a [String],
    pub exclude_filenames: bool,
    pub reranker: &'a str,
    pub frequency_search: bool,
    pub max_results: Option<usize>,
    pub max_bytes: Option<usize>,
    pub max_tokens: Option<usize>,
    pub allow_tests: bool,
    pub exact: bool,
    pub no_merge: bool,
    pub merge_threshold: Option<usize>,
    pub dry_run: bool,
    pub session: Option<&'a str>,
}
