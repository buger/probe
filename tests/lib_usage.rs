#[cfg(test)]
mod tests {
    use probe_code::search::{perform_probe, SearchOptions};
    use std::path::Path;

    #[test]
    fn test_search_functionality() {
        // Create search options
        let options = SearchOptions {
            path: Path::new("."),
            queries: &["function".to_string()],
            files_only: false,
            custom_ignores: &[],
            exclude_filenames: false,
            reranker: "bm25",
            frequency_search: true,
            exact: false,
            language: None,
            max_results: Some(5),
            max_bytes: None,
            max_tokens: None,
            allow_tests: true,
            no_merge: false,
            merge_threshold: None,
            dry_run: false,
            session: None,
            timeout: 30,
            question: None,
            no_gitignore: false,
            lsp: false,
        };

        let results = perform_probe(&options).unwrap();

        // Just check that we get some results
        assert!(!results.results.is_empty());
        println!("Found {} results", results.results.len());
    }

    #[test]
    fn test_query_functionality() {
        use probe_code::query::{perform_query, QueryOptions};

        let options = QueryOptions {
            path: Path::new("."),
            pattern: "fn",
            language: Some("rust"),
            ignore: &[],
            allow_tests: true,
            max_results: Some(5),
            format: "text",
            no_gitignore: false,
        };

        let matches = perform_query(&options).unwrap();

        // Just check that we get some results
        assert!(!matches.is_empty());
        println!("Found {} matches", matches.len());
    }
}
