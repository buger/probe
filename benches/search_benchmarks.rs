use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use probe_code::search::{perform_probe, SearchOptions};
use std::fs;
use tempfile::TempDir;

/// Create a temporary directory with test files for benchmarking
fn create_test_codebase() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path();

    // Create different file types with varying complexity
    let test_files = vec![
        (
            "src/main.rs",
            r#"
use std::collections::HashMap;
use std::io;

fn main() {
    println!("Hello, world!");
    let mut map = HashMap::new();
    map.insert("key", "value");
    
    for i in 0..1000 {
        process_item(i);
    }
}

fn process_item(item: i32) -> Result<(), io::Error> {
    if item % 2 == 0 {
        println!("Even: {}", item);
    } else {
        println!("Odd: {}", item);
    }
    Ok(())
}

struct DataProcessor {
    data: Vec<i32>,
    processed: bool,
}

impl DataProcessor {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            processed: false,
        }
    }
    
    fn process(&mut self) {
        self.data.sort();
        self.processed = true;
    }
}
"#,
        ),
        (
            "src/lib.rs",
            r#"
pub mod utils;
pub mod search;
pub mod parser;

pub use search::*;
pub use parser::*;

/// Main library functionality
pub fn run_search(query: &str, path: &str) -> Vec<String> {
    let mut results = Vec::new();
    
    // Simulate search logic
    for i in 0..100 {
        if i % 10 == 0 {
            results.push(format!("Result {}: {}", i, query));
        }
    }
    
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_search() {
        let results = run_search("test", ".");
        assert!(!results.is_empty());
    }
}
"#,
        ),
        (
            "src/utils.rs",
            r#"
use std::collections::HashMap;

pub fn calculate_metrics(data: &[i32]) -> HashMap<String, f64> {
    let mut metrics = HashMap::new();
    
    if data.is_empty() {
        return metrics;
    }
    
    let sum: i32 = data.iter().sum();
    let avg = sum as f64 / data.len() as f64;
    let min = *data.iter().min().unwrap();
    let max = *data.iter().max().unwrap();
    
    metrics.insert("average".to_string(), avg);
    metrics.insert("min".to_string(), min as f64);
    metrics.insert("max".to_string(), max as f64);
    metrics.insert("sum".to_string(), sum as f64);
    
    metrics
}

pub fn process_text(text: &str) -> String {
    text.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn fibonacci(n: usize) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
"#,
        ),
        (
            "src/parser.rs",
            r#"
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Token {
    Identifier(String),
    Number(i32),
    String(String),
    Operator(String),
    Keyword(String),
}

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, position: 0 }
    }
    
    pub fn parse(&mut self) -> Result<HashMap<String, Token>, String> {
        let mut symbols = HashMap::new();
        
        while self.position < self.tokens.len() {
            match &self.tokens[self.position] {
                Token::Identifier(name) => {
                    symbols.insert(name.clone(), self.tokens[self.position].clone());
                }
                Token::Keyword(keyword) => {
                    if keyword == "function" {
                        self.parse_function(&mut symbols)?;
                    }
                }
                _ => {}
            }
            self.position += 1;
        }
        
        Ok(symbols)
    }
    
    fn parse_function(&mut self, symbols: &mut HashMap<String, Token>) -> Result<(), String> {
        self.position += 1;
        if self.position < self.tokens.len() {
            if let Token::Identifier(name) = &self.tokens[self.position] {
                symbols.insert(format!("function_{}", name), self.tokens[self.position].clone());
            }
        }
        Ok(())
    }
}

pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let words: Vec<&str> = input.split_whitespace().collect();
    
    for word in words {
        if word.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(num) = word.parse::<i32>() {
                tokens.push(Token::Number(num));
            }
        } else if word.starts_with('"') && word.ends_with('"') {
            tokens.push(Token::String(word[1..word.len()-1].to_string()));
        } else if ["fn", "struct", "impl", "use", "mod"].contains(&word) {
            tokens.push(Token::Keyword(word.to_string()));
        } else if ["+", "-", "*", "/", "=", "==", "!="].contains(&word) {
            tokens.push(Token::Operator(word.to_string()));
        } else {
            tokens.push(Token::Identifier(word.to_string()));
        }
    }
    
    tokens
}
"#,
        ),
        (
            "tests/integration_tests.rs",
            r#"
use probe_code::*;

#[test]
fn test_basic_search() {
    let results = run_search("main", "src/");
    assert!(!results.is_empty());
}

#[test]
fn test_complex_search() {
    let results = run_search("HashMap", "src/");
    assert!(!results.is_empty());
}

#[test]
fn test_function_search() {
    let results = run_search("process_item", "src/");
    assert!(!results.is_empty());
}

#[test]
fn test_struct_search() {
    let results = run_search("DataProcessor", "src/");
    assert!(!results.is_empty());
}

#[test]
fn test_empty_search() {
    let results = run_search("nonexistent_term", "src/");
    assert!(results.is_empty());
}
"#,
        ),
        (
            "README.md",
            r#"
# Test Project

This is a test project for benchmarking the probe search tool.

## Features

- Fast search capabilities
- AST-based parsing
- Multiple language support
- Efficient indexing

## Usage

```bash
probe "search term" /path/to/code
```

## Architecture

The system consists of several key components:

1. **Parser** - Tokenizes and parses code
2. **Search Engine** - Performs semantic search
3. **Indexer** - Builds efficient search indices
4. **CLI** - Command-line interface

## Performance

The tool is designed for high performance with:

- Parallel processing using Rayon
- Efficient data structures
- Optimized search algorithms
- Memory-efficient parsing

## Examples

Search for function definitions:
```bash
probe "fn main" src/
```

Search for struct usage:
```bash
probe "HashMap" src/
```

Search for specific patterns:
```bash
probe "process_item" src/
```
"#,
        ),
    ];

    // Create directory structure
    fs::create_dir_all(base_path.join("src")).unwrap();
    fs::create_dir_all(base_path.join("tests")).unwrap();

    // Write test files
    for (path, content) in test_files {
        fs::write(base_path.join(path), content).unwrap();
    }

    temp_dir
}

/// Benchmark different search patterns
fn benchmark_search_patterns(c: &mut Criterion) {
    let temp_dir = create_test_codebase();
    let search_path = temp_dir.path().to_path_buf();

    let test_patterns = vec![
        ("simple_word", "main"),
        ("function_call", "process_item"),
        ("type_usage", "HashMap"),
        ("struct_def", "DataProcessor"),
        ("keyword", "fn"),
        ("common_word", "use"),
        ("rare_word", "fibonacci"),
        ("number", "1000"),
        ("empty_result", "nonexistent_xyz"),
    ];

    let mut group = c.benchmark_group("search_patterns");

    for (name, pattern) in test_patterns {
        group.bench_with_input(BenchmarkId::new("pattern", name), &pattern, |b, pattern| {
            b.iter(|| {
                let query = vec![pattern.to_string()];
                let options = SearchOptions {
                    path: &search_path,
                    queries: &query,
                    files_only: false,
                    custom_ignores: &[],
                    exclude_filenames: false,
                    reranker: "hybrid",
                    frequency_search: true,
                    exact: false,
                    language: None,
                    max_results: Some(100),
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

                black_box(perform_probe(&options).unwrap())
            })
        });
    }

    group.finish();
}

/// Benchmark different result limits
fn benchmark_result_limits(c: &mut Criterion) {
    let temp_dir = create_test_codebase();
    let search_path = temp_dir.path().to_path_buf();

    let limits = vec![1, 10, 50, 100, 500];
    let mut group = c.benchmark_group("result_limits");

    for limit in limits {
        group.bench_with_input(BenchmarkId::new("limit", limit), &limit, |b, &limit| {
            b.iter(|| {
                let query = vec!["use".to_string()]; // Common pattern
                let options = SearchOptions {
                    path: &search_path,
                    queries: &query,
                    files_only: false,
                    custom_ignores: &[],
                    exclude_filenames: false,
                    reranker: "hybrid",
                    frequency_search: true,
                    exact: false,
                    language: None,
                    max_results: Some(limit),
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

                black_box(perform_probe(&options).unwrap())
            })
        });
    }

    group.finish();
}

/// Benchmark different search options
fn benchmark_search_options(c: &mut Criterion) {
    let temp_dir = create_test_codebase();
    let search_path = temp_dir.path().to_path_buf();

    let mut group = c.benchmark_group("search_options");

    // Test different rerankers
    let rerankers = vec!["hybrid", "frequency", "semantic"];
    for reranker in rerankers {
        group.bench_with_input(
            BenchmarkId::new("reranker", reranker),
            &reranker,
            |b, &reranker| {
                b.iter(|| {
                    let query = vec!["HashMap".to_string()];
                    let options = SearchOptions {
                        path: &search_path,
                        queries: &query,
                        files_only: false,
                        custom_ignores: &[],
                        exclude_filenames: false,
                        reranker,
                        frequency_search: true,
                        exact: false,
                        language: None,
                        max_results: Some(50),
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

                    black_box(perform_probe(&options).unwrap())
                })
            },
        );
    }

    // Test with/without frequency search
    group.bench_with_input(
        BenchmarkId::new("frequency", "enabled"),
        &true,
        |b, &freq| {
            b.iter(|| {
                let query = vec!["HashMap".to_string()];
                let options = SearchOptions {
                    path: &search_path,
                    queries: &query,
                    files_only: false,
                    custom_ignores: &[],
                    exclude_filenames: false,
                    reranker: "hybrid",
                    frequency_search: freq,
                    exact: false,
                    language: None,
                    max_results: Some(50),
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

                black_box(perform_probe(&options).unwrap())
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("frequency", "disabled"),
        &false,
        |b, &freq| {
            b.iter(|| {
                let query = vec!["HashMap".to_string()];
                let options = SearchOptions {
                    path: &search_path,
                    queries: &query,
                    files_only: false,
                    custom_ignores: &[],
                    exclude_filenames: false,
                    reranker: "hybrid",
                    frequency_search: freq,
                    exact: false,
                    language: None,
                    max_results: Some(50),
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

                black_box(perform_probe(&options).unwrap())
            })
        },
    );

    group.finish();
}

/// Benchmark query complexity
fn benchmark_query_complexity(c: &mut Criterion) {
    let temp_dir = create_test_codebase();
    let search_path = temp_dir.path().to_path_buf();

    let queries = vec![
        ("simple", vec!["main"]),
        ("compound", vec!["HashMap", "Vec"]),
        (
            "complex",
            vec!["process_item", "DataProcessor", "fibonacci"],
        ),
        ("multi_term", vec!["fn", "struct", "impl", "use"]),
    ];

    let mut group = c.benchmark_group("query_complexity");

    for (name, query_terms) in queries {
        group.bench_with_input(
            BenchmarkId::new("complexity", name),
            &query_terms,
            |b, query_terms| {
                b.iter(|| {
                    let query: Vec<String> = query_terms.iter().map(|s| s.to_string()).collect();
                    let options = SearchOptions {
                        path: &search_path,
                        queries: &query,
                        files_only: false,
                        custom_ignores: &[],
                        exclude_filenames: false,
                        reranker: "hybrid",
                        frequency_search: true,
                        exact: false,
                        language: None,
                        max_results: Some(100),
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

                    black_box(perform_probe(&options).unwrap())
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_search_patterns,
    benchmark_result_limits,
    benchmark_search_options,
    benchmark_query_complexity
);
criterion_main!(benches);
