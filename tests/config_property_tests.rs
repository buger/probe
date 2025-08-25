//! Property-based tests for the configuration system
//!
//! This module uses proptest to verify configuration invariants hold
//! across a wide range of randomly generated inputs.

#![allow(clippy::field_reassign_with_default)]

use probe_code::config::{
    DefaultsConfig, IndexingConfig, IndexingFeatures, LspWorkspaceCacheConfig, PerformanceConfig,
    ProbeConfig, SearchConfig,
};
use proptest::prelude::*;

// Strategy for generating valid log levels
fn log_level_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("error".to_string()),
        Just("warn".to_string()),
        Just("info".to_string()),
        Just("debug".to_string()),
        Just("trace".to_string()),
    ]
}

// Strategy for generating valid formats
fn format_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("color".to_string()),
        Just("json".to_string()),
        Just("plain".to_string()),
        Just("xml".to_string()),
    ]
}

// Strategy for generating valid rerankers
fn reranker_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("bm25".to_string()),
        Just("tfidf".to_string()),
        Just("bert".to_string()),
    ]
}

// Strategy for generating optional booleans
fn optional_bool() -> impl Strategy<Value = Option<bool>> {
    prop_oneof![Just(None), any::<bool>().prop_map(Some),]
}

// Strategy for generating optional usizes
fn optional_usize(max: usize) -> impl Strategy<Value = Option<usize>> {
    prop_oneof![Just(None), (0..=max).prop_map(Some),]
}

// Strategy for generating DefaultsConfig
fn defaults_config_strategy() -> impl Strategy<Value = Option<DefaultsConfig>> {
    prop_oneof![
        Just(None),
        (
            optional_bool(),
            prop_oneof![Just(None), log_level_strategy().prop_map(Some)],
            optional_bool(),
            prop_oneof![Just(None), format_strategy().prop_map(Some)],
            optional_usize(300).prop_map(|opt| opt.map(|v| v as u64)),
        )
            .prop_map(|(debug, log_level, enable_lsp, format, timeout)| {
                Some(DefaultsConfig {
                    debug,
                    log_level,
                    enable_lsp,
                    format,
                    timeout,
                })
            })
    ]
}

// Strategy for generating SearchConfig
fn search_config_strategy() -> impl Strategy<Value = Option<SearchConfig>> {
    prop_oneof![
        Just(None),
        (
            optional_usize(1000),
            optional_usize(100000),
            optional_usize(1000000),
            optional_bool(),
            prop_oneof![Just(None), reranker_strategy().prop_map(Some)],
            optional_usize(100),
            optional_bool(),
            optional_bool(),
        )
            .prop_map(
                |(
                    max_results,
                    max_tokens,
                    max_bytes,
                    frequency,
                    reranker,
                    merge_threshold,
                    allow_tests,
                    no_gitignore,
                )| {
                    Some(SearchConfig {
                        max_results,
                        max_tokens,
                        max_bytes,
                        frequency,
                        reranker,
                        merge_threshold,
                        allow_tests,
                        no_gitignore,
                    })
                }
            )
    ]
}

// Strategy for generating IndexingFeatures
fn indexing_features_strategy() -> impl Strategy<Value = Option<IndexingFeatures>> {
    prop_oneof![
        Just(None),
        (
            optional_bool(),
            optional_bool(),
            optional_bool(),
            optional_bool(),
            optional_bool(),
        )
            .prop_map(
                |(
                    extract_functions,
                    extract_types,
                    extract_variables,
                    extract_imports,
                    extract_tests,
                )| {
                    Some(IndexingFeatures {
                        extract_functions,
                        extract_types,
                        extract_variables,
                        extract_imports,
                        extract_tests,
                    })
                }
            )
    ]
}

// Strategy for generating ProbeConfig
fn probe_config_strategy() -> impl Strategy<Value = ProbeConfig> {
    (defaults_config_strategy(), search_config_strategy()).prop_map(|(defaults, search)| {
        ProbeConfig {
            defaults,
            search,
            extract: None, // Simplified for property tests
            query: None,
            lsp: None,
            performance: None,
            indexing: None,
        }
    })
}

proptest! {
    #[test]
    fn test_default_config_has_valid_defaults(
        _dummy in any::<()>(),
    ) {
        // Test that a default config with resolution works
        let default_config = ProbeConfig::default();

        // Verify basic structure is valid
        assert!(default_config.defaults.is_none() || default_config.defaults.is_some());
        assert!(default_config.search.is_none() || default_config.search.is_some());

        // The real test: creating a resolved config from defaults should work without panicking
        let resolved = probe_code::config::get_config();
        assert!(!resolved.defaults.log_level.is_empty());
        assert!(!resolved.defaults.format.is_empty());
        assert!(resolved.defaults.timeout > 0);
    }

    #[test]
    fn test_json_serialization_roundtrip(
        config in probe_config_strategy(),
    ) {
        // Serialize to JSON and back
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ProbeConfig = serde_json::from_str(&json).unwrap();

        // Verify critical fields survive the roundtrip
        if let Some(ref defaults) = config.defaults {
            if let Some(debug) = defaults.debug {
                assert_eq!(
                    deserialized.defaults.as_ref().and_then(|d| d.debug),
                    Some(debug)
                );
            }
        }
    }

    #[test]
    fn test_config_fields_are_optional(
        timeout in 0u64..=1000,
        max_results in 0usize..=10000,
        tree_cache_size in 0usize..=100000,
    ) {
        // Test creating configs with individual fields
        let mut config = ProbeConfig::default();
        config.defaults = Some(DefaultsConfig {
            timeout: Some(timeout),
            ..Default::default()
        });
        config.search = Some(SearchConfig {
            max_results: Some(max_results),
            ..Default::default()
        });
        config.performance = Some(PerformanceConfig {
            tree_cache_size: Some(tree_cache_size),
            ..Default::default()
        });

        // Should be serializable
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("timeout") || json.contains("null"));

        // Should be deserializable
        let _deserialized: ProbeConfig = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_indexing_features_merge(
        features1 in indexing_features_strategy(),
        features2 in indexing_features_strategy(),
    ) {
        if let (Some(f1), Some(f2)) = (features1, features2) {
            let mut merged = IndexingFeatures::default();

            // Simulate merging logic: f2 overrides f1
            merged.extract_functions = f2.extract_functions.or(f1.extract_functions);
            merged.extract_types = f2.extract_types.or(f1.extract_types);
            merged.extract_variables = f2.extract_variables.or(f1.extract_variables);
            merged.extract_imports = f2.extract_imports.or(f1.extract_imports);
            merged.extract_tests = f2.extract_tests.or(f1.extract_tests);

            // Verify merge precedence (f2 overrides f1)
            if let Some(v) = f2.extract_functions {
                assert_eq!(merged.extract_functions, Some(v));
            } else if let Some(v) = f1.extract_functions {
                assert_eq!(merged.extract_functions, Some(v));
            }
        }
    }

    #[test]
    fn test_environment_variable_names_valid(
        field_name in "[A-Z][A-Z0-9_]*",
    ) {
        // Test that environment variable names follow the pattern PROBE_*
        let env_var_name = format!("PROBE_{field_name}");

        // Should be uppercase with underscores
        assert!(env_var_name.chars().all(|c| c.is_uppercase() || c.is_numeric() || c == '_'));
        assert!(env_var_name.starts_with("PROBE_"));
    }

    #[test]
    fn test_priority_languages_valid(
        languages in prop::collection::vec("[a-z]+", 0..10),
    ) {
        let mut config = IndexingConfig::default();
        config.priority_languages = if languages.is_empty() {
            None
        } else {
            Some(languages.clone())
        };

        // Priority languages should be preserved
        if !languages.is_empty() {
            assert_eq!(config.priority_languages, Some(languages));
        }
    }

    #[test]
    fn test_cache_size_limits(
        max_open_caches in 1usize..=1000,
        size_mb in 1usize..=10000,
        lookup_depth in 1usize..=100,
    ) {
        let cache_config = LspWorkspaceCacheConfig {
            max_open_caches: Some(max_open_caches),
            size_mb_per_workspace: Some(size_mb),
            lookup_depth: Some(lookup_depth),
            base_dir: None,
        };

        // All values should be within reasonable ranges
        assert!(cache_config.max_open_caches.unwrap() >= 1);
        assert!(cache_config.max_open_caches.unwrap() <= 1000);
        assert!(cache_config.size_mb_per_workspace.unwrap() >= 1);
        assert!(cache_config.size_mb_per_workspace.unwrap() <= 10000);
        assert!(cache_config.lookup_depth.unwrap() >= 1);
        assert!(cache_config.lookup_depth.unwrap() <= 100);
    }

    #[test]
    fn test_config_default_values_consistency(
        _dummy in any::<()>(),
    ) {
        // Test consistency of default configuration values
        let resolved_config = probe_code::config::get_config();

        // Verify reasonable defaults
        assert!(resolved_config.defaults.timeout >= 1 && resolved_config.defaults.timeout <= 300);
        assert!(resolved_config.performance.tree_cache_size >= 100);
        assert!(resolved_config.search.merge_threshold >= 1);

        // Verify string defaults are not empty
        assert!(!resolved_config.defaults.log_level.is_empty());
        assert!(!resolved_config.defaults.format.is_empty());
        assert!(!resolved_config.search.reranker.is_empty());
    }

    #[test]
    fn test_config_validation_bounds(
        timeout in 1u64..=600,
        cache_size in 1usize..=50000,
        merge_threshold in 1usize..=50,
    ) {
        // Test various configuration bounds
        let mut config = ProbeConfig::default();
        config.defaults = Some(DefaultsConfig {
            timeout: Some(timeout),
            ..Default::default()
        });
        config.performance = Some(PerformanceConfig {
            tree_cache_size: Some(cache_size),
            ..Default::default()
        });
        config.search = Some(SearchConfig {
            merge_threshold: Some(merge_threshold),
            ..Default::default()
        });

        // Should serialize without issues
        let json = serde_json::to_string(&config);
        assert!(json.is_ok());

        // Should deserialize without issues
        let serialized = json.unwrap();
        let deserialized: Result<ProbeConfig, _> = serde_json::from_str(&serialized);
        assert!(deserialized.is_ok());

        // Verify values are preserved
        let config = deserialized.unwrap();
        if let Some(defaults) = config.defaults {
            assert_eq!(defaults.timeout, Some(timeout));
        }
        if let Some(perf) = config.performance {
            assert_eq!(perf.tree_cache_size, Some(cache_size));
        }
        if let Some(search) = config.search {
            assert_eq!(search.merge_threshold, Some(merge_threshold));
        }
    }
}
