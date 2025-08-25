use probe_code::lsp_integration::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use probe_code::lsp_integration::client::LspClient;
use probe_code::lsp_integration::types::{CallHierarchyInfo, LspConfig, NodeKey};
use probe_code::utils::hash::md5_hex_file;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[tokio::main]
async fn main() {
    println!("=== Manual LSP Call Graph Cache Test ===\n");

    // Create cache with visible TTL for testing
    let cache = Arc::new(CallGraphCache::new(CallGraphCacheConfig {
        ttl: std::time::Duration::from_secs(300), // 5 minutes for testing
        ..Default::default()
    }));

    // Test file in the probe codebase itself
    let test_file = PathBuf::from("/Users/leonidbugaev/conductor/repo/probe/paris/src/lsp_integration/client.rs");
    let symbol_name = "get_symbol_info";
    let line = 300; // Approximate line number
    let column = 12;

    println!("Testing with file: {}", test_file.display());
    println!("Symbol: {} at {}:{}\n", symbol_name, line, column);

    // First call - should fetch from LSP
    println!("=== First Call (Cold Cache) ===");
    let start = Instant::now();
    
    let content_md5 = md5_hex_file(&test_file).unwrap();
    let key = NodeKey::new(symbol_name, test_file.clone(), content_md5.clone());
    
    let cache_clone = cache.clone();
    let test_file_clone = test_file.clone();
    let symbol_name_clone = symbol_name.to_string();
    
    let result = cache
        .get_or_compute(key.clone(), move || {
            let file = test_file_clone.clone();
            let symbol = symbol_name_clone.clone();
            async move {
                println!("  🔄 Computing call hierarchy via LSP...");
                
                // Create LSP client
                let config = LspConfig {
                    use_daemon: true,
                    workspace_hint: Some("/Users/leonidbugaev/conductor/repo/probe/paris".to_string()),
                    timeout_ms: 30000,
                };
                
                let client = LspClient::new(config).await
                    .ok_or_else(|| anyhow::anyhow!("Failed to create LSP client"))?;
                
                // Get symbol info with call hierarchy
                let symbol_info = client
                    .get_symbol_info(&file, &symbol, line, column)
                    .await?;
                
                if let Some(info) = symbol_info {
                    if let Some(hierarchy) = info.call_hierarchy {
                        println!("  ✅ Got hierarchy from LSP");
                        return Ok(hierarchy);
                    }
                }
                
                // Return empty hierarchy if none found
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            }
        })
        .await
        .unwrap();
    
    let elapsed = start.elapsed();
    println!("  ⏱️  Time taken: {:.2?}", elapsed);
    println!("  📥 Incoming calls: {}", result.info.incoming_calls.len());
    println!("  📤 Outgoing calls: {}", result.info.outgoing_calls.len());
    
    // Print some details
    if !result.info.incoming_calls.is_empty() {
        println!("\n  Incoming from:");
        for call in &result.info.incoming_calls[..3.min(result.info.incoming_calls.len())] {
            println!("    - {} ({}:{})", call.name, call.file_path, call.line);
        }
    }
    
    if !result.info.outgoing_calls.is_empty() {
        println!("\n  Calls to:");
        for call in &result.info.outgoing_calls[..3.min(result.info.outgoing_calls.len())] {
            println!("    - {} ({}:{})", call.name, call.file_path, call.line);
        }
    }
    
    println!("\n=== Second Call (Warm Cache) ===");
    let start = Instant::now();
    
    // Same key, should hit cache
    let cached = cache.get(&key);
    let elapsed = start.elapsed();
    
    if let Some(cached_node) = cached {
        println!("  ✅ Cache HIT!");
        println!("  ⏱️  Time taken: {:.2?} (immediate!)", elapsed);
        println!("  📥 Incoming calls: {}", cached_node.info.incoming_calls.len());
        println!("  📤 Outgoing calls: {}", cached_node.info.outgoing_calls.len());
    } else {
        println!("  ❌ Cache MISS (unexpected)");
    }
    
    // Simulate file modification
    println!("\n=== Simulating File Modification ===");
    println!("  📝 File would be modified here (changing MD5)...");
    println!("  🔄 Creating new key with different content hash...");
    
    let modified_key = NodeKey::new(
        symbol_name,
        test_file.clone(),
        format!("{}_modified", content_md5), // Simulate different MD5
    );
    
    let start = Instant::now();
    let cache_clone = cache.clone();
    let test_file_clone = test_file.clone();
    let symbol_name_clone = symbol_name.to_string();
    
    let result = cache_clone
        .get_or_compute(modified_key.clone(), move || {
            let file = test_file_clone.clone();
            let symbol = symbol_name_clone.clone();
            async move {
                println!("  🔄 Re-computing due to content change...");
                
                // Simulate LSP call
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                
                Ok(CallHierarchyInfo {
                    incoming_calls: vec![],
                    outgoing_calls: vec![],
                })
            }
        })
        .await
        .unwrap();
    
    let elapsed = start.elapsed();
    println!("  ⏱️  Time taken: {:.2?} (recomputed due to MD5 change)", elapsed);
    
    // Test invalidation
    println!("\n=== Testing File Invalidation ===");
    cache.invalidate_file(&test_file);
    println!("  🗑️  Invalidated all entries for file");
    
    let cached = cache.get(&key);
    if cached.is_none() {
        println!("  ✅ Original key successfully invalidated");
    } else {
        println!("  ❌ Key still in cache (unexpected)");
    }
    
    // Show cache stats
    println!("\n=== Cache Statistics ===");
    let stats = cache.stats();
    println!("  📊 Total cached nodes: {}", stats.total_nodes);
    println!("  📊 Total node IDs: {}", stats.total_ids);
    println!("  📊 Total files tracked: {}", stats.total_files);
    println!("  📊 Total edges: {}", stats.total_edges);
    println!("  📊 In-flight computations: {}", stats.inflight_computations);
    
    println!("\n✅ Manual test completed!");
}