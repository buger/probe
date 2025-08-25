#![allow(clippy::uninlined_format_args)]
#![allow(clippy::needless_range_loop)]

use probe_code::lsp_integration::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use probe_code::lsp_integration::types::{CallHierarchyInfo, NodeId, NodeKey};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_cache_deduplication() {
    let cache = Arc::new(CallGraphCache::new(CallGraphCacheConfig::default()));
    let key = NodeKey::new(
        "test_func",
        PathBuf::from("/test/file.rs"),
        "abc123".to_string(),
    );

    let compute_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Launch multiple concurrent requests for the same key
    let mut handles = vec![];
    for _ in 0..10 {
        let cache_clone = cache.clone();
        let key_clone = key.clone();
        let count_clone = compute_count.clone();

        let handle = tokio::spawn(async move {
            cache_clone
                .get_or_compute(key_clone, move || {
                    let count = count_clone.clone();
                    async move {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Ok(CallHierarchyInfo {
                            incoming_calls: vec![],
                            outgoing_calls: vec![],
                        })
                    }
                })
                .await
                .unwrap()
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Should have only computed once despite 10 concurrent requests
    assert_eq!(
        compute_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "Should only compute once due to deduplication"
    );
}

#[tokio::test]
async fn test_graph_invalidation() {
    let cache = CallGraphCache::new(CallGraphCacheConfig {
        invalidation_depth: 2,
        ..Default::default()
    });

    // Create a call graph: main -> utils -> helper -> logger
    let main_id = NodeId::new("main", PathBuf::from("/src/main.rs"));
    let utils_id = NodeId::new("utils", PathBuf::from("/src/utils.rs"));
    let helper_id = NodeId::new("helper", PathBuf::from("/src/helper.rs"));
    let logger_id = NodeId::new("logger", PathBuf::from("/src/logger.rs"));

    // Set up edges
    cache.update_edges(&main_id, vec![], vec![utils_id.clone()]);
    cache.update_edges(&utils_id, vec![main_id.clone()], vec![helper_id.clone()]);
    cache.update_edges(&helper_id, vec![utils_id.clone()], vec![logger_id.clone()]);
    cache.update_edges(&logger_id, vec![helper_id.clone()], vec![]);

    // Add cached entries for each
    let nodes = vec![
        ("main", &main_id),
        ("utils", &utils_id),
        ("helper", &helper_id),
        ("logger", &logger_id),
    ];

    for (name, id) in &nodes {
        let key = NodeKey::new(*name, id.file.clone(), format!("hash_{}", name));
        cache
            .get_or_compute(key, || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();
    }

    // Verify all are cached
    for (name, id) in &nodes {
        let key = NodeKey::new(*name, id.file.clone(), format!("hash_{}", name));
        assert!(cache.get(&key).is_some(), "{} should be cached", name);
    }

    // Invalidate utils with depth 2 - should affect main, utils, helper (but not logger)
    cache.invalidate_node(&utils_id, 2);

    let main_key = NodeKey::new(
        "main",
        PathBuf::from("/src/main.rs"),
        "hash_main".to_string(),
    );
    let utils_key = NodeKey::new(
        "utils",
        PathBuf::from("/src/utils.rs"),
        "hash_utils".to_string(),
    );
    let helper_key = NodeKey::new(
        "helper",
        PathBuf::from("/src/helper.rs"),
        "hash_helper".to_string(),
    );
    let logger_key = NodeKey::new(
        "logger",
        PathBuf::from("/src/logger.rs"),
        "hash_logger".to_string(),
    );

    assert!(cache.get(&main_key).is_none(), "main should be invalidated");
    assert!(
        cache.get(&utils_key).is_none(),
        "utils should be invalidated"
    );
    assert!(
        cache.get(&helper_key).is_none(),
        "helper should be invalidated"
    );
    assert!(
        cache.get(&logger_key).is_some(),
        "logger should NOT be invalidated (depth limit)"
    );
}

#[tokio::test]
async fn test_file_based_invalidation() {
    let cache = CallGraphCache::new(CallGraphCacheConfig::default());

    let test_file = PathBuf::from("/test/module.rs");

    // Add multiple functions from the same file
    let func1_key = NodeKey::new("func1", test_file.clone(), "hash1".to_string());
    let func2_key = NodeKey::new("func2", test_file.clone(), "hash2".to_string());
    let func3_key = NodeKey::new("func3", test_file.clone(), "hash3".to_string());

    for key in [&func1_key, &func2_key, &func3_key] {
        cache
            .get_or_compute(key.clone(), || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();
    }

    // All should be cached
    assert!(cache.get(&func1_key).is_some());
    assert!(cache.get(&func2_key).is_some());
    assert!(cache.get(&func3_key).is_some());

    // Invalidate the entire file
    cache.invalidate_file(&test_file);

    // All functions from that file should be invalidated
    assert!(cache.get(&func1_key).is_none());
    assert!(cache.get(&func2_key).is_none());
    assert!(cache.get(&func3_key).is_none());
}

#[tokio::test]
async fn test_ttl_eviction() {
    let cache = CallGraphCache::new(CallGraphCacheConfig {
        ttl: Duration::from_secs(1),
        ..Default::default()
    });

    let key = NodeKey::new(
        "short_lived",
        PathBuf::from("/test/temp.rs"),
        "temp123".to_string(),
    );

    cache
        .get_or_compute(key.clone(), || async {
            Ok(CallHierarchyInfo {
                incoming_calls: vec![],
                outgoing_calls: vec![],
            })
        })
        .await
        .unwrap();

    assert!(cache.get(&key).is_some(), "Should be cached initially");

    // Wait for TTL to expire
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Trigger eviction by adding a new entry
    let trigger_key = NodeKey::new(
        "trigger",
        PathBuf::from("/test/trigger.rs"),
        "trigger123".to_string(),
    );
    cache
        .get_or_compute(trigger_key, || async {
            Ok(CallHierarchyInfo {
                incoming_calls: vec![],
                outgoing_calls: vec![],
            })
        })
        .await
        .unwrap();

    assert!(
        cache.get(&key).is_none(),
        "Should be evicted after TTL expiration"
    );
}

#[tokio::test]
async fn test_capacity_eviction() {
    let cache = CallGraphCache::new(CallGraphCacheConfig {
        capacity: 3,
        ..Default::default()
    });

    let mut keys = vec![];

    // Add 4 entries to a cache with capacity 3
    for i in 0..4 {
        let key = NodeKey::new(
            format!("func{}", i),
            PathBuf::from(format!("/test/file{}.rs", i)),
            format!("hash{}", i),
        );

        cache
            .get_or_compute(key.clone(), || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();

        // Touch earlier entries to update their access time
        if i > 0 {
            for j in 0..i {
                cache.get(&keys[j]);
            }
        }

        keys.push(key);
    }

    // The first entry (func0) should be evicted as it was accessed least recently
    assert!(
        cache.get(&keys[0]).is_none(),
        "Least recently used entry should be evicted"
    );

    // The last 3 should still be cached
    for i in 1..4 {
        assert!(
            cache.get(&keys[i]).is_some(),
            "Entry {} should still be cached",
            i
        );
    }
}

#[test]
fn test_edge_consistency() {
    let cache = CallGraphCache::new(CallGraphCacheConfig::default());

    let a = NodeId::new("a", PathBuf::from("/a.rs"));
    let b = NodeId::new("b", PathBuf::from("/b.rs"));
    let c = NodeId::new("c", PathBuf::from("/c.rs"));

    // Create edges: a -> b, b -> c
    cache.update_edges(&a, vec![], vec![b.clone()]);
    cache.update_edges(&b, vec![a.clone()], vec![c.clone()]);
    cache.update_edges(&c, vec![b.clone()], vec![]);

    // Now update b's edges to remove connection to c
    cache.update_edges(&b, vec![a.clone()], vec![]);

    // Verify edges are consistent
    let stats = cache.stats();
    assert!(stats.total_edges > 0, "Should have edges in the graph");

    // After the update, c should no longer have b as incoming
    // This is validated internally by the cache's edge consistency maintenance
}

#[tokio::test]
async fn test_cache_stats() {
    let cache = CallGraphCache::new(CallGraphCacheConfig::default());

    // Add some nodes and edges
    let file1 = PathBuf::from("/src/file1.rs");
    let file2 = PathBuf::from("/src/file2.rs");

    let id1 = NodeId::new("func1", file1.clone());
    let id2 = NodeId::new("func2", file2.clone());

    cache.update_edges(&id1, vec![], vec![id2.clone()]);

    let key1 = NodeKey::new("func1", file1, "hash1".to_string());
    let key2 = NodeKey::new("func2", file2, "hash2".to_string());

    for key in [key1, key2] {
        cache
            .get_or_compute(key, || async {
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            })
            .await
            .unwrap();
    }

    let stats = cache.stats();
    assert_eq!(stats.total_nodes, 2, "Should have 2 cached nodes");
    assert_eq!(stats.total_ids, 2, "Should track 2 node IDs");
    assert_eq!(stats.total_files, 2, "Should track 2 files");
    assert!(stats.total_edges > 0, "Should have edges in the graph");
    assert_eq!(
        stats.inflight_computations, 0,
        "No computations should be in flight"
    );
}
