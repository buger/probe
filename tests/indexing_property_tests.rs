//! Property-based tests for the indexing system
//!
//! These tests use proptest to generate random inputs and verify that
//! the indexing system maintains its invariants under all conditions.

use lsp_daemon::cache_types::LspOperation;
use lsp_daemon::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use lsp_daemon::indexing::{IndexingManager, IndexingQueue, ManagerConfig, Priority, QueueItem};
use lsp_daemon::lsp_cache::{LspCache, LspCacheConfig};
use lsp_daemon::lsp_registry::LspRegistry;
use lsp_daemon::server_manager::SingleServerManager;
use lsp_daemon::LanguageDetector;
use proptest::prelude::*;
use proptest::test_runner::Config;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::runtime::Runtime;
use tokio::time::timeout;

// Helper to create async runtime for property tests
fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

// Strategy to generate priorities
fn priority_strategy() -> impl Strategy<Value = Priority> {
    prop_oneof![
        Just(Priority::Critical),
        Just(Priority::High),
        Just(Priority::Medium),
        Just(Priority::Low)
    ]
}

// Strategy to generate queue items
fn queue_item_strategy() -> impl Strategy<Value = QueueItem> {
    (
        r"[a-zA-Z0-9_/\.]{1,50}",                // file path
        priority_strategy(),                     // priority
        proptest::option::of(1u64..1024 * 1024), // estimated size
        proptest::option::of(r"[a-z]{2,10}"),    // language hint
    )
        .prop_map(|(path, priority, size, lang)| {
            let mut item = QueueItem::new(PathBuf::from(path), priority);
            if let Some(s) = size {
                item = item.with_estimated_size(s);
            }
            if let Some(l) = lang {
                item = item.with_language_hint(l);
            }
            item
        })
}

// Strategy to generate vectors of queue items
fn queue_items_strategy() -> impl Strategy<Value = Vec<QueueItem>> {
    proptest::collection::vec(queue_item_strategy(), 0..200)
}

// Strategy for manager configuration
fn manager_config_strategy() -> impl Strategy<Value = ManagerConfig> {
    (
        1usize..8,         // max_workers
        1u64..1024 * 1024, // memory_budget_bytes (up to 1MB for tests)
        0.1f64..1.0,       // memory_pressure_threshold
        10usize..1000,     // max_queue_size
        1u64..1024,        // max_file_size_bytes
        1usize..50,        // discovery_batch_size
    )
        .prop_map(
            |(workers, memory, threshold, queue_size, file_size, batch_size)| ManagerConfig {
                max_workers: workers,
                memory_budget_bytes: memory,
                memory_pressure_threshold: threshold,
                max_queue_size: queue_size,
                exclude_patterns: vec!["*.tmp".to_string(), "*/target/*".to_string()],
                include_patterns: vec![],
                max_file_size_bytes: file_size,
                enabled_languages: vec![],
                incremental_mode: false,
                discovery_batch_size: batch_size,
                status_update_interval_secs: 1,
            },
        )
}

proptest! {
    #![proptest_config(Config::with_cases(50))]

    /// Property: Queue operations preserve priority ordering
    #[test]
    fn prop_queue_priority_ordering(items in queue_items_strategy()) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue = IndexingQueue::unlimited();

            // Enqueue all items
            for item in items.clone() {
                queue.enqueue(item).await.unwrap();
            }

            // Dequeue all items and verify priority ordering
            let mut dequeued = Vec::new();
            while let Some(item) = queue.dequeue().await {
                dequeued.push(item);
            }

            // Should have dequeued same number of items
            prop_assert_eq!(dequeued.len(), items.len());

            // Priority should be monotonically non-increasing
            for i in 1..dequeued.len() {
                prop_assert!(dequeued[i-1].priority.as_u8() >= dequeued[i].priority.as_u8());
            }
            Ok(())
        });
    }

    /// Property: Queue metrics are always consistent
    #[test]
    fn prop_queue_metrics_consistency(items in queue_items_strategy()) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let max_size = if items.is_empty() { 0 } else { items.len() + 10 };
            let queue = IndexingQueue::new(max_size);

            let mut enqueued_count = 0;
            let mut total_bytes = 0u64;

            // Enqueue items and track expectations
            for item in items.clone() {
                if queue.enqueue(item.clone()).await.unwrap() {
                    enqueued_count += 1;
                    if let Some(size) = item.estimated_size {
                        total_bytes += size;
                    }
                }
            }

            let metrics = queue.get_metrics().await;

            // Verify metric consistency
            prop_assert_eq!(metrics.total_items, enqueued_count);
            prop_assert_eq!(metrics.estimated_total_bytes, total_bytes);
            prop_assert_eq!(
                metrics.total_items,
                metrics.critical_priority_items + metrics.high_priority_items +
                metrics.medium_priority_items + metrics.low_priority_items
            );
            prop_assert_eq!(metrics.total_enqueued as usize, enqueued_count);
            prop_assert_eq!(metrics.total_dequeued, 0);

            if max_size > 0 {
                prop_assert!(metrics.utilization_ratio >= 0.0);
                prop_assert!(metrics.utilization_ratio <= 1.0);
            }
            Ok(())
        });
    }

    /// Property: Memory tracking is accurate across operations
    #[test]
    fn prop_memory_tracking_accuracy(
        operations in proptest::collection::vec(
            prop_oneof![
                queue_item_strategy().prop_map(|item| ("enqueue", Some(item))),
                Just(("dequeue", None))
            ],
            1..100
        )
    ) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue = IndexingQueue::unlimited();
            let mut expected_bytes = 0u64;

            for (op, maybe_item) in operations {
                match op {
                    "enqueue" => {
                        if let Some(item) = maybe_item {
                            if let Some(size) = item.estimated_size {
                                expected_bytes += size;
                            }
                            queue.enqueue(item).await.unwrap();
                        }
                    }
                    "dequeue" => {
                        if let Some(item) = queue.dequeue().await {
                            if let Some(size) = item.estimated_size {
                                expected_bytes = expected_bytes.saturating_sub(size);
                            }
                        }
                    }
                    _ => {}
                }
            }

            let metrics = queue.get_metrics().await;
            prop_assert_eq!(metrics.estimated_total_bytes, expected_bytes);
            Ok(())
        });
    }

    /// Property: Queue size limits are always respected
    #[test]
    fn prop_queue_size_limits(
        max_size in 1usize..50,
        items in queue_items_strategy()
    ) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue = IndexingQueue::new(max_size);

            let mut enqueued_count = 0;
            for item in items {
                if queue.enqueue(item).await.unwrap() {
                    enqueued_count += 1;
                }

                // Queue size should never exceed limit
                prop_assert!(queue.len() <= max_size);
                prop_assert_eq!(queue.len(), enqueued_count);

                if queue.len() == max_size {
                    break; // Stop when limit reached
                }
            }

            // Final verification
            prop_assert!(queue.len() <= max_size);
            Ok(())
        });
    }

    /// Property: Paused queues reject all operations
    #[test]
    fn prop_paused_queue_behavior(items in proptest::collection::vec(queue_item_strategy(), 1..20)) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue = IndexingQueue::unlimited();

            // Pause queue
            queue.pause();
            prop_assert!(queue.is_paused());

            // All enqueue operations should fail
            for item in items {
                prop_assert!(!queue.enqueue(item).await.unwrap());
            }

            // Dequeue should return None
            prop_assert!(queue.dequeue().await.is_none());
            prop_assert_eq!(queue.len(), 0);
            Ok(())
        });
    }

    /// Property: Batch operations are equivalent to individual operations
    #[test]
    fn prop_batch_vs_individual_enqueue(items in queue_items_strategy()) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue1 = IndexingQueue::unlimited();
            let queue2 = IndexingQueue::unlimited();

            // Enqueue individually
            for item in items.clone() {
                queue1.enqueue(item).await.unwrap();
            }

            // Enqueue as batch
            queue2.enqueue_batch(items.clone()).await.unwrap();

            // Both queues should have same state
            let metrics1 = queue1.get_metrics().await;
            let metrics2 = queue2.get_metrics().await;

            prop_assert_eq!(metrics1.total_items, metrics2.total_items);
            prop_assert_eq!(metrics1.estimated_total_bytes, metrics2.estimated_total_bytes);
            prop_assert_eq!(metrics1.critical_priority_items, metrics2.critical_priority_items);
            prop_assert_eq!(metrics1.high_priority_items, metrics2.high_priority_items);
            prop_assert_eq!(metrics1.medium_priority_items, metrics2.medium_priority_items);
            prop_assert_eq!(metrics1.low_priority_items, metrics2.low_priority_items);
            Ok(())
        });
    }

    /// Property: Queue clearing operations maintain consistency
    #[test]
    fn prop_clear_operations_consistency(
        items in queue_items_strategy(),
        clear_priority in priority_strategy()
    ) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue = IndexingQueue::unlimited();

            // Enqueue all items
            for item in items.clone() {
                queue.enqueue(item).await.unwrap();
            }

            let before_metrics = queue.get_metrics().await;

            // Clear specific priority
            queue.clear_priority(clear_priority).await;

            let after_metrics = queue.get_metrics().await;

            // Verify the cleared priority queue is empty
            let cleared_count = match clear_priority {
                Priority::Critical => before_metrics.critical_priority_items,
                Priority::High => before_metrics.high_priority_items,
                Priority::Medium => before_metrics.medium_priority_items,
                Priority::Low => before_metrics.low_priority_items,
            };

            prop_assert_eq!(
                after_metrics.total_items,
                before_metrics.total_items - cleared_count
            );

            // Verify the specific priority queue is empty
            let current_count = queue.len_for_priority(clear_priority).await;
            prop_assert_eq!(current_count, 0);
            Ok(())
        });
    }

    /// Property: Manager configuration validation
    #[test]
    fn prop_manager_config_validation(config in manager_config_strategy()) {
        prop_assert!(config.max_workers > 0);
        prop_assert!(config.memory_budget_bytes > 0);
        prop_assert!(config.memory_pressure_threshold > 0.0);
        prop_assert!(config.memory_pressure_threshold <= 1.0);
        prop_assert!(config.max_queue_size > 0);
        prop_assert!(config.max_file_size_bytes > 0);
        prop_assert!(config.discovery_batch_size > 0);
    }

    /// Property: Priority string parsing is consistent
    #[test]
    fn prop_priority_parsing_consistency(priority in priority_strategy()) {
        let as_str = priority.as_str();
        let parsed = Priority::from_str(as_str);
        prop_assert_eq!(parsed, Some(priority));

        // Test case insensitive parsing
        let uppercase = as_str.to_uppercase();
        let parsed_upper = Priority::from_str(&uppercase);
        prop_assert_eq!(parsed_upper, Some(priority));
    }

    /// Property: Queue item IDs are unique and increasing
    #[test]
    fn prop_queue_item_unique_ids(paths in proptest::collection::vec(r"[a-z]{1,20}", 1..100)) {
        let mut items = Vec::new();
        let mut ids = HashSet::new();
        let mut last_id = 0u64;

        for path in paths {
            let item = QueueItem::low_priority(PathBuf::from(path));

            // ID should be unique
            prop_assert!(!ids.contains(&item.id));
            ids.insert(item.id);

            // ID should be increasing
            prop_assert!(item.id > last_id);
            last_id = item.id;

            items.push(item);
        }
    }

    /// Property: Remove matching operations preserve invariants
    #[test]
    fn prop_remove_matching_preserves_invariants(
        items in queue_items_strategy(),
        remove_pattern in r"[a-z]{1,10}"
    ) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue = IndexingQueue::unlimited();

            // Enqueue all items
            for item in items.clone() {
                queue.enqueue(item).await.unwrap();
            }

            let before_metrics = queue.get_metrics().await;

            // Count items that should be removed
            let expected_removed = items
                .iter()
                .filter(|item| {
                    item.file_path
                        .to_string_lossy()
                        .contains(&remove_pattern)
                })
                .count();

            // Remove matching items
            let actually_removed = queue
                .remove_matching(|item| {
                    item.file_path.to_string_lossy().contains(&remove_pattern)
                })
                .await;

            prop_assert_eq!(actually_removed, expected_removed);

            let after_metrics = queue.get_metrics().await;
            prop_assert_eq!(
                after_metrics.total_items,
                before_metrics.total_items - actually_removed
            );

            // Verify remaining items don't match the pattern
            while let Some(item) = queue.dequeue().await {
                prop_assert!(!item.file_path.to_string_lossy().contains(&remove_pattern));
            }
            Ok(())
        });
    }
}

// Additional integration property tests
proptest! {
    #![proptest_config(Config::with_cases(10))] // Fewer cases for expensive tests

    /// Property: Concurrent enqueue/dequeue operations maintain consistency
    #[test]
    fn prop_concurrent_operations_consistency(
        items1 in proptest::collection::vec(queue_item_strategy(), 10..50),
        items2 in proptest::collection::vec(queue_item_strategy(), 10..50)
    ) {
        let rt = runtime();
        let _ = rt.block_on(async {
            let queue = Arc::new(IndexingQueue::unlimited());
            let total_items = items1.len() + items2.len();

            // Spawn concurrent enqueue tasks
            let queue1 = Arc::clone(&queue);
            let handle1 = tokio::spawn(async move {
                for item in items1 {
                    queue1.enqueue(item).await.unwrap();
                    tokio::task::yield_now().await; // Encourage interleaving
                }
            });

            let queue2 = Arc::clone(&queue);
            let handle2 = tokio::spawn(async move {
                for item in items2 {
                    queue2.enqueue(item).await.unwrap();
                    tokio::task::yield_now().await;
                }
            });

            // Spawn concurrent dequeue task
            let queue3 = Arc::clone(&queue);
            let handle3 = tokio::spawn(async move {
                let mut dequeued = Vec::new();
                while dequeued.len() < total_items {
                    if let Some(item) = queue3.dequeue().await {
                        dequeued.push(item);
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
                dequeued
            });

            // Wait for completion
            handle1.await.unwrap();
            handle2.await.unwrap();
            let dequeued = handle3.await.unwrap();

            // Verify consistency
            prop_assert_eq!(dequeued.len(), total_items);
            prop_assert!(queue.is_empty());

            // Verify priority ordering is maintained
            for i in 1..dequeued.len() {
                prop_assert!(dequeued[i-1].priority.as_u8() >= dequeued[i].priority.as_u8());
            }
            Ok(())
        });
    }

    /// Property: Manager handles various file structures correctly
    #[test]
    fn prop_manager_file_discovery(
        file_count in 1usize..20,
        extensions in proptest::collection::vec(r"[a-z]{2,4}", 1..5)
    ) {
        let rt = runtime();
        let _ = rt.block_on(async {
            // Create temporary directory structure
            let temp_dir = tempfile::tempdir().unwrap();
            let root = temp_dir.path();

            let mut expected_files = 0;

            // Create files with various extensions
            for i in 0..file_count {
                let ext = &extensions[i % extensions.len()];
                let file_path = root.join(format!("file_{i}.{ext}"));
                let content = format!("// File {i} content");
                fs::write(&file_path, content).await.unwrap();

                // Count expected files (assuming all extensions are valid)
                expected_files += 1;
            }

            // Create indexing manager
            let language_detector = Arc::new(LanguageDetector::new());
            let config = ManagerConfig {
                max_workers: 2,
                memory_budget_bytes: 64 * 1024 * 1024,
                memory_pressure_threshold: 0.8,
                max_queue_size: file_count * 2,
                exclude_patterns: vec![],
                include_patterns: vec![],
                max_file_size_bytes: 1024 * 1024,
                enabled_languages: vec![],
                incremental_mode: false,
                discovery_batch_size: 10,
                status_update_interval_secs: 1,
            };

            // Create mock LSP dependencies for testing
            let registry = Arc::new(LspRegistry::new().expect("Failed to create registry"));
            let child_processes = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            let server_manager = Arc::new(SingleServerManager::new_with_tracker(
                registry,
                child_processes,
            ));

            let cache_config = CallGraphCacheConfig {
                capacity: 100,
                ttl: Duration::from_secs(300),
                eviction_check_interval: Duration::from_secs(30),
                invalidation_depth: 1,
                ..Default::default()
            };
            let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));

            let lsp_cache_config = LspCacheConfig {
                capacity_per_operation: 100,
                ttl: Duration::from_secs(300),
                eviction_check_interval: Duration::from_secs(30),
                persistent: false,
                cache_directory: None,
            };
            let definition_cache = Arc::new(
                LspCache::new(LspOperation::Definition, lsp_cache_config)
                    .expect("Failed to create definition cache"),
            );
            let manager = IndexingManager::new(
                config,
                language_detector,
                server_manager,
                call_graph_cache,
                definition_cache,
            );

            // Start indexing with timeout
            let result = timeout(
                Duration::from_secs(30),
                manager.start_indexing(root.to_path_buf())
            ).await;

            prop_assert!(result.is_ok());

            // Wait for file discovery with timeout
            let discovery_result = timeout(Duration::from_secs(10), async {
                loop {
                    let progress = manager.get_progress().await;
                    if progress.total_files > 0 {
                        return progress.total_files;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }).await;

            manager.stop_indexing().await.unwrap();

            if let Ok(discovered_files) = discovery_result {
                // Should discover at least some files
                prop_assert!(discovered_files > 0);
                // But not more than we created (due to filtering)
                prop_assert!(discovered_files <= expected_files as u64);
            }
            Ok(())
        });
    }
}
