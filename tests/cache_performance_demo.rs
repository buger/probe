#![allow(clippy::uninlined_format_args)]

use probe_code::lsp_integration::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use probe_code::lsp_integration::types::{CallHierarchyInfo, CallInfo, NodeKey};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
async fn test_cache_performance_demonstration() {
    println!("\n=== Cache Performance Demonstration ===\n");

    let cache = Arc::new(CallGraphCache::new(CallGraphCacheConfig::default()));
    let file_path = PathBuf::from("/src/main.rs");
    let symbol = "main";
    let content_md5 = "abc123".to_string();

    // Simulate expensive LSP call
    let compute_expensive = || async {
        println!("  🔄 Simulating expensive LSP call (500ms delay)...");
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(CallHierarchyInfo {
            incoming_calls: vec![
                CallInfo {
                    name: "test_runner".to_string(),
                    file_path: "/src/test.rs".to_string(),
                    line: 10,
                    column: 5,
                    symbol_kind: "function".to_string(),
                },
                CallInfo {
                    name: "benchmark".to_string(),
                    file_path: "/src/bench.rs".to_string(),
                    line: 20,
                    column: 8,
                    symbol_kind: "function".to_string(),
                },
            ],
            outgoing_calls: vec![
                CallInfo {
                    name: "init".to_string(),
                    file_path: "/src/lib.rs".to_string(),
                    line: 1,
                    column: 0,
                    symbol_kind: "function".to_string(),
                },
                CallInfo {
                    name: "run".to_string(),
                    file_path: "/src/app.rs".to_string(),
                    line: 50,
                    column: 4,
                    symbol_kind: "function".to_string(),
                },
            ],
        })
    };

    // First call - should be slow
    println!("1. First call (cold cache):");
    let key = NodeKey::new(symbol, file_path.clone(), content_md5.clone());

    let start = Instant::now();
    let result1 = cache
        .get_or_compute(key.clone(), compute_expensive)
        .await
        .unwrap();
    let elapsed1 = start.elapsed();

    println!("   ✅ Completed in {:?}", elapsed1);
    println!("   📥 {} incoming calls", result1.info.incoming_calls.len());
    println!("   📤 {} outgoing calls", result1.info.outgoing_calls.len());
    assert!(
        elapsed1.as_millis() >= 500,
        "First call should take at least 500ms"
    );

    // Second call - should be instant
    println!("\n2. Second call (warm cache):");
    let start = Instant::now();
    let result2 = cache.get(&key).unwrap();
    let elapsed2 = start.elapsed();

    println!("   ✅ Completed in {:?} (from cache!)", elapsed2);
    println!("   📥 {} incoming calls", result2.info.incoming_calls.len());
    println!("   📤 {} outgoing calls", result2.info.outgoing_calls.len());
    assert!(elapsed2.as_millis() < 10, "Cache hit should be under 10ms");

    // Performance comparison
    let speedup = elapsed1.as_micros() as f64 / elapsed2.as_micros().max(1) as f64;
    println!("\n3. Performance Summary:");
    println!("   ⚡ Speedup: {:.0}x faster", speedup);
    println!("   ⏱️  First call: {:?}", elapsed1);
    println!("   ⏱️  Cached call: {:?}", elapsed2);
    println!("   💾 Memory saved: 1 LSP roundtrip avoided");

    // Simulate file change
    println!("\n4. After file modification (different MD5):");
    let modified_key = NodeKey::new(symbol, file_path.clone(), "def456".to_string());

    let start = Instant::now();
    let _result3 = cache
        .get_or_compute(modified_key, compute_expensive)
        .await
        .unwrap();
    let elapsed3 = start.elapsed();

    println!(
        "   ✅ Recomputed in {:?} (cache miss due to content change)",
        elapsed3
    );
    assert!(
        elapsed3.as_millis() >= 500,
        "Modified file should trigger recomputation"
    );

    // Test concurrent access
    println!("\n5. Concurrent access test (10 parallel requests):");
    let key = NodeKey::new(
        "concurrent_test",
        PathBuf::from("/test.rs"),
        "hash123".to_string(),
    );
    let cache_clone = cache.clone();

    let compute_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let start = Instant::now();

    let mut handles = vec![];
    for i in 0..10 {
        let cache = cache_clone.clone();
        let key = key.clone();
        let count = compute_count.clone();

        let handle = tokio::spawn(async move {
            cache
                .get_or_compute(key, move || {
                    let c = count.clone();
                    async move {
                        c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Ok(CallHierarchyInfo {
                            incoming_calls: vec![],
                            outgoing_calls: vec![],
                        })
                    }
                })
                .await
                .unwrap();
            i
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let elapsed_concurrent = start.elapsed();
    let actual_computes = compute_count.load(std::sync::atomic::Ordering::SeqCst);

    println!("   ✅ 10 requests completed in {:?}", elapsed_concurrent);
    println!(
        "   🔒 Only {} actual computation(s) (deduplication working!)",
        actual_computes
    );
    assert_eq!(
        actual_computes, 1,
        "Should only compute once despite concurrent requests"
    );

    // Final stats
    println!("\n6. Cache Statistics:");
    let stats = cache.stats();
    println!("   📊 Total cached nodes: {}", stats.total_nodes);
    println!("   📊 Total unique symbols: {}", stats.total_ids);
    println!("   📊 Files tracked: {}", stats.total_files);

    println!("\n✅ Performance demonstration complete!");
    println!("   The cache provides massive speedups for repeated LSP queries!");
}
