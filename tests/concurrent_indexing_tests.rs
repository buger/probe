//! Concurrent access and thread safety tests for indexing components
//!
//! These tests verify that all indexing components are thread-safe and handle
//! concurrent access correctly without data races, deadlocks, or corruption.

use anyhow::Result;
use lsp_daemon::cache_types::LspOperation;
use lsp_daemon::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use lsp_daemon::indexing::{
    IndexingManager, IndexingProgress, IndexingQueue, ManagerConfig, Priority, QueueItem,
};
use lsp_daemon::lsp_cache::{LspCache, LspCacheConfig};
use lsp_daemon::lsp_registry::LspRegistry;
use lsp_daemon::server_manager::SingleServerManager;
use lsp_daemon::LanguageDetector;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::fs;
use tokio::sync::Barrier;
use tokio::time::sleep;

/// Helper for creating concurrent test workspaces
struct ConcurrentTestWorkspace {
    #[allow(dead_code)]
    temp_dir: TempDir, // Keeps the temp directory alive
    root_path: PathBuf,
}

impl ConcurrentTestWorkspace {
    async fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let root_path = temp_dir.path().to_path_buf();
        fs::create_dir_all(root_path.join("src")).await?;
        Ok(Self {
            temp_dir,
            root_path,
        })
    }

    fn path(&self) -> &std::path::Path {
        &self.root_path
    }

    async fn create_files(&self, count: usize) -> Result<()> {
        for i in 0..count {
            let content = format!(
                r#"
// File {i} for concurrent testing
pub fn function_{i}() -> i32 {{
    let value_{i} = {i};
    println!("Processing item {{}}", value_{i});
    value_{i}
}}

pub struct Struct{i} {{
    field: i32,
}}

impl Struct{i} {{
    pub fn new() -> Self {{
        Self {{ field: {i} }}
    }}
    
    pub fn get_field(&self) -> i32 {{
        self.field
    }}
    
    pub fn set_field(&mut self, value: i32) {{
        self.field = value;
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn test_function_{i}() {{
        assert_eq!(function_{i}(), {i});
    }}
    
    #[test]
    fn test_struct_{i}() {{
        let mut s = Struct{i}::new();
        assert_eq!(s.get_field(), {i});
        s.set_field(42);
        assert_eq!(s.get_field(), 42);
    }}
}}
"#
            );

            let file_path = self.root_path.join(format!("src/file_{i:04}.rs"));
            fs::write(file_path, content).await?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn test_queue_concurrent_enqueue_dequeue() -> Result<()> {
    const NUM_PRODUCERS: usize = 10;
    const NUM_CONSUMERS: usize = 5;
    const ITEMS_PER_PRODUCER: usize = 100;

    let queue = Arc::new(IndexingQueue::unlimited());
    let enqueued_count = Arc::new(AtomicUsize::new(0));
    let dequeued_count = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(NUM_PRODUCERS + NUM_CONSUMERS));

    let mut handles = Vec::new();

    // Spawn producer tasks
    for producer_id in 0..NUM_PRODUCERS {
        let queue_clone = Arc::clone(&queue);
        let enqueued_count_clone = Arc::clone(&enqueued_count);
        let barrier_clone = Arc::clone(&barrier);

        let handle = tokio::spawn(async move {
            // Wait for all tasks to be ready
            barrier_clone.wait().await;

            for item_id in 0..ITEMS_PER_PRODUCER {
                let path = format!("/concurrent/producer_{producer_id}/item_{item_id}.rs");
                let priority = match item_id % 4 {
                    0 => Priority::Critical,
                    1 => Priority::High,
                    2 => Priority::Medium,
                    _ => Priority::Low,
                };

                let item = QueueItem::new(PathBuf::from(path), priority)
                    .with_estimated_size((item_id as u64 + 1) * 1024)
                    .with_language_hint("rust".to_string());

                if queue_clone.enqueue(item).await.unwrap() {
                    enqueued_count_clone.fetch_add(1, Ordering::Relaxed);
                }

                // Yield to encourage interleaving
                if item_id % 10 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });

        handles.push(handle);
    }

    // Spawn consumer tasks
    for consumer_id in 0..NUM_CONSUMERS {
        let queue_clone = Arc::clone(&queue);
        let dequeued_count_clone = Arc::clone(&dequeued_count);
        let barrier_clone = Arc::clone(&barrier);

        let handle = tokio::spawn(async move {
            // Wait for all tasks to be ready
            barrier_clone.wait().await;

            let mut local_dequeued = 0;
            loop {
                match queue_clone.dequeue().await {
                    Some(_item) => {
                        local_dequeued += 1;
                        dequeued_count_clone.fetch_add(1, Ordering::Relaxed);

                        // Simulate some processing time
                        if local_dequeued % 20 == 0 {
                            tokio::task::yield_now().await;
                        }
                    }
                    None => {
                        // No more items, yield and check if we're done
                        tokio::task::yield_now().await;

                        // Stop if we've dequeued all expected items
                        if dequeued_count_clone.load(Ordering::Relaxed)
                            >= NUM_PRODUCERS * ITEMS_PER_PRODUCER
                        {
                            break;
                        }
                    }
                }
            }

            println!("Consumer {consumer_id} dequeued {local_dequeued} items");
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await?;
    }

    // Verify results
    let final_enqueued = enqueued_count.load(Ordering::Relaxed);
    let final_dequeued = dequeued_count.load(Ordering::Relaxed);

    assert_eq!(final_enqueued, NUM_PRODUCERS * ITEMS_PER_PRODUCER);
    assert_eq!(final_dequeued, NUM_PRODUCERS * ITEMS_PER_PRODUCER);
    assert!(queue.is_empty());

    let final_metrics = queue.get_metrics().await;
    assert_eq!(final_metrics.total_enqueued as usize, final_enqueued);
    assert_eq!(final_metrics.total_dequeued as usize, final_dequeued);

    println!("Concurrent queue test: {final_enqueued} enqueued, {final_dequeued} dequeued");

    Ok(())
}

#[tokio::test]
async fn test_progress_concurrent_updates() -> Result<()> {
    const NUM_WORKERS: usize = 8;
    const UPDATES_PER_WORKER: usize = 1000;

    let progress = Arc::new(IndexingProgress::new());
    let barrier = Arc::new(Barrier::new(NUM_WORKERS));
    let mut handles = Vec::new();

    // Initialize progress with some totals
    progress.add_total_files((NUM_WORKERS * UPDATES_PER_WORKER) as u64);

    for worker_id in 0..NUM_WORKERS {
        let progress_clone = Arc::clone(&progress);
        let barrier_clone = Arc::clone(&barrier);

        let handle = tokio::spawn(async move {
            // Wait for all workers to be ready
            barrier_clone.wait().await;

            progress_clone.add_worker();

            for update_id in 0..UPDATES_PER_WORKER {
                // Simulate different types of updates
                match update_id % 4 {
                    0 => {
                        progress_clone.start_file();
                        progress_clone.complete_file(1024, 5);
                    }
                    1 => {
                        progress_clone.start_file();
                        progress_clone.fail_file(&format!("Worker {worker_id} error {update_id}"));
                    }
                    2 => {
                        progress_clone.skip_file("Already processed");
                    }
                    3 => {
                        progress_clone.update_memory_usage((worker_id * 1024 * 1024) as u64);
                    }
                    _ => unreachable!(),
                }

                // Yield occasionally
                if update_id % 100 == 0 {
                    tokio::task::yield_now().await;
                }
            }

            progress_clone.remove_worker();
        });

        handles.push(handle);
    }

    // Wait for all workers to complete
    for handle in handles {
        handle.await?;
    }

    // Verify final state
    let final_snapshot = progress.get_snapshot();

    assert_eq!(final_snapshot.active_workers, 0);
    assert_eq!(
        final_snapshot.processed_files + final_snapshot.failed_files + final_snapshot.skipped_files,
        final_snapshot.total_files
    );
    assert!(final_snapshot.is_complete());

    println!(
        "Concurrent progress test - Processed: {}, Failed: {}, Skipped: {}",
        final_snapshot.processed_files, final_snapshot.failed_files, final_snapshot.skipped_files
    );

    Ok(())
}

#[tokio::test]
async fn test_manager_concurrent_start_stop() -> Result<()> {
    let workspace = ConcurrentTestWorkspace::new().await?;
    workspace.create_files(20).await?;

    const NUM_OPERATIONS: usize = 10;

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 2,
        memory_budget_bytes: 64 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 100,
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
    let manager = Arc::new(IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    ));
    let successful_starts = Arc::new(AtomicUsize::new(0));
    let successful_stops = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();

    // Spawn tasks that try to start/stop concurrently
    for operation_id in 0..NUM_OPERATIONS {
        let manager_clone = Arc::clone(&manager);
        let workspace_path = workspace.path().to_path_buf();
        let successful_starts_clone = Arc::clone(&successful_starts);
        let successful_stops_clone = Arc::clone(&successful_stops);

        let handle = tokio::spawn(async move {
            // Random delay to spread out operations
            sleep(Duration::from_millis((operation_id * 50) as u64)).await;

            if operation_id % 2 == 0 {
                // Try to start indexing
                match manager_clone.start_indexing(workspace_path).await {
                    Ok(_) => {
                        successful_starts_clone.fetch_add(1, Ordering::Relaxed);
                        println!("Operation {operation_id}: Started indexing");

                        // Let it run for a bit
                        sleep(Duration::from_millis(200)).await;
                    }
                    Err(e) => {
                        println!("Operation {operation_id}: Failed to start: {e}");
                    }
                }
            } else {
                // Try to stop indexing
                match manager_clone.stop_indexing().await {
                    Ok(_) => {
                        successful_stops_clone.fetch_add(1, Ordering::Relaxed);
                        println!("Operation {operation_id}: Stopped indexing");
                    }
                    Err(e) => {
                        println!("Operation {operation_id}: Failed to stop: {e}");
                    }
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all operations to complete
    for handle in handles {
        handle.await?;
    }

    // Ensure final cleanup
    let _ = manager.stop_indexing().await;

    let starts = successful_starts.load(Ordering::Relaxed);
    let stops = successful_stops.load(Ordering::Relaxed);

    println!("Concurrent start/stop test: {starts} successful starts, {stops} successful stops");

    // Should have at least one successful start
    assert!(starts > 0, "Expected at least one successful start");

    Ok(())
}

#[tokio::test]
async fn test_queue_stress_with_size_limits() -> Result<()> {
    const MAX_QUEUE_SIZE: usize = 100;
    const NUM_PRODUCERS: usize = 20;
    const ITEMS_PER_PRODUCER: usize = 50;

    let queue = Arc::new(IndexingQueue::new(MAX_QUEUE_SIZE));
    let rejected_count = Arc::new(AtomicUsize::new(0));
    let accepted_count = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();

    // Spawn many producers trying to fill the queue
    for producer_id in 0..NUM_PRODUCERS {
        let queue_clone = Arc::clone(&queue);
        let rejected_count_clone = Arc::clone(&rejected_count);
        let accepted_count_clone = Arc::clone(&accepted_count);

        let handle = tokio::spawn(async move {
            for item_id in 0..ITEMS_PER_PRODUCER {
                let path = format!("/stress/producer_{producer_id}/item_{item_id}.rs");
                let item = QueueItem::high_priority(PathBuf::from(path));

                match queue_clone.enqueue(item).await {
                    Ok(true) => {
                        accepted_count_clone.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(false) => {
                        rejected_count_clone.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        rejected_count_clone.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Yield occasionally
                if item_id % 10 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });

        handles.push(handle);
    }

    // Spawn a consumer to drain some items
    let queue_consumer = Arc::clone(&queue);
    let consumer_handle = tokio::spawn(async move {
        let mut consumed = 0;
        while consumed < 500 {
            if let Some(_item) = queue_consumer.dequeue().await {
                consumed += 1;
            } else {
                tokio::task::yield_now().await;
            }
        }
        consumed
    });

    // Wait for all producers
    for handle in handles {
        handle.await?;
    }

    let consumed = consumer_handle.await?;

    let final_accepted = accepted_count.load(Ordering::Relaxed);
    let final_rejected = rejected_count.load(Ordering::Relaxed);

    // Verify queue size constraints were respected
    assert!(queue.len() <= MAX_QUEUE_SIZE);

    println!(
        "Queue stress test: {} accepted, {} rejected, {} consumed, final queue size: {}",
        final_accepted,
        final_rejected,
        consumed,
        queue.len()
    );

    // Total attempted should equal accepted + rejected
    assert_eq!(
        final_accepted + final_rejected,
        NUM_PRODUCERS * ITEMS_PER_PRODUCER
    );

    Ok(())
}

#[tokio::test]
async fn test_concurrent_queue_operations_mixed() -> Result<()> {
    const NUM_TASKS: usize = 20;
    const OPERATIONS_PER_TASK: usize = 200;

    let queue = Arc::new(IndexingQueue::new(500));
    let operation_counts = Arc::new([
        AtomicUsize::new(0), // enqueue
        AtomicUsize::new(0), // dequeue
        AtomicUsize::new(0), // peek
        AtomicUsize::new(0), // clear
        AtomicUsize::new(0), // metrics
    ]);

    let mut handles = Vec::new();

    for task_id in 0..NUM_TASKS {
        let queue_clone = Arc::clone(&queue);
        let operation_counts_clone = Arc::clone(&operation_counts);

        let handle = tokio::spawn(async move {
            for op_id in 0..OPERATIONS_PER_TASK {
                match op_id % 5 {
                    0 => {
                        // Enqueue
                        let item = QueueItem::medium_priority(PathBuf::from(format!(
                            "/mixed/task_{task_id}/op_{op_id}.rs"
                        )));
                        if queue_clone.enqueue(item).await.unwrap_or(false) {
                            operation_counts_clone[0].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    1 => {
                        // Dequeue
                        if queue_clone.dequeue().await.is_some() {
                            operation_counts_clone[1].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    2 => {
                        // Peek
                        if queue_clone.peek().await.is_some() {
                            operation_counts_clone[2].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    3 => {
                        // Clear priority (occasionally)
                        if op_id % 50 == 0 {
                            queue_clone.clear_priority(Priority::Low).await;
                            operation_counts_clone[3].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    4 => {
                        // Get metrics
                        let _metrics = queue_clone.get_metrics().await;
                        operation_counts_clone[4].fetch_add(1, Ordering::Relaxed);
                    }
                    _ => unreachable!(),
                }

                // Yield occasionally
                if op_id % 25 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await?;
    }

    let enqueues = operation_counts[0].load(Ordering::Relaxed);
    let dequeues = operation_counts[1].load(Ordering::Relaxed);
    let peeks = operation_counts[2].load(Ordering::Relaxed);
    let clears = operation_counts[3].load(Ordering::Relaxed);
    let metrics_calls = operation_counts[4].load(Ordering::Relaxed);

    println!(
        "Mixed operations test: {enqueues} enqueues, {dequeues} dequeues, {peeks} peeks, {clears} clears, {metrics_calls} metrics"
    );

    // Verify queue is in consistent state
    let final_metrics = queue.get_metrics().await;
    assert_eq!(final_metrics.total_items, queue.len());

    Ok(())
}

#[tokio::test]
async fn test_manager_worker_statistics_thread_safety() -> Result<()> {
    let workspace = ConcurrentTestWorkspace::new().await?;
    workspace.create_files(30).await?;

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 4,
        memory_budget_bytes: 128 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 200,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 1024 * 1024,
        enabled_languages: vec!["Rust".to_string()],
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
    let manager = Arc::new(IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    ));
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Concurrently access worker statistics while indexing is running
    let stat_access_count = Arc::new(AtomicUsize::new(0));
    let mut stat_handles = Vec::new();

    for reader_id in 0..8 {
        let manager_clone = Arc::clone(&manager);
        let stat_access_count_clone = Arc::clone(&stat_access_count);

        let handle = tokio::spawn(async move {
            while stat_access_count_clone.load(Ordering::Relaxed) < 500 {
                // Access various statistics
                let _progress = manager_clone.get_progress().await;
                let _queue_snapshot = manager_clone.get_queue_snapshot().await;
                let _worker_stats = manager_clone.get_worker_stats().await;
                let _status = manager_clone.get_status().await;

                stat_access_count_clone.fetch_add(1, Ordering::Relaxed);

                if reader_id % 2 == 0 {
                    sleep(Duration::from_millis(10)).await;
                } else {
                    tokio::task::yield_now().await;
                }
            }
        });

        stat_handles.push(handle);
    }

    // Let statistics reading run while indexing progresses
    let start_time = Instant::now();
    while start_time.elapsed() < Duration::from_secs(8) {
        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    // Wait for all statistics readers to finish
    for handle in stat_handles {
        handle.await?;
    }

    let final_progress = manager.get_progress().await;
    let final_worker_stats = manager.get_worker_stats().await;

    manager.stop_indexing().await?;

    println!(
        "Concurrent statistics access: {} progress calls, final: {} files processed",
        stat_access_count.load(Ordering::Relaxed),
        final_progress.processed_files
    );

    // Verify statistics are consistent
    assert_eq!(final_worker_stats.len(), 4); // Should have 4 workers
    let total_worker_files: u64 = final_worker_stats.iter().map(|s| s.files_processed).sum();

    // Worker stats might not exactly match progress due to timing, but should be reasonably close
    if final_progress.processed_files > 0 {
        assert!(total_worker_files > 0);
    }

    Ok(())
}

#[tokio::test]
async fn test_memory_tracking_thread_safety() -> Result<()> {
    const NUM_UPDATERS: usize = 10;
    const UPDATES_PER_UPDATER: usize = 1000;

    let progress = Arc::new(IndexingProgress::new());
    let total_expected = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::new();

    for updater_id in 0..NUM_UPDATERS {
        let progress_clone = Arc::clone(&progress);
        let total_expected_clone = Arc::clone(&total_expected);

        let handle = tokio::spawn(async move {
            let mut local_total = 0u64;

            for update_id in 0..UPDATES_PER_UPDATER {
                let memory_change = ((updater_id * 1000) + update_id) as u64;

                // Update memory usage
                progress_clone.update_memory_usage(memory_change);
                local_total = memory_change; // Last value will be the final memory

                // Yield occasionally
                if update_id % 100 == 0 {
                    tokio::task::yield_now().await;
                }
            }

            total_expected_clone.fetch_add(local_total, Ordering::Relaxed);
        });

        handles.push(handle);
    }

    // Wait for all updaters
    for handle in handles {
        handle.await?;
    }

    let final_snapshot = progress.get_snapshot();

    // Memory tracking should be consistent (exact value depends on timing)
    assert!(final_snapshot.memory_usage_bytes > 0);
    assert!(final_snapshot.peak_memory_bytes >= final_snapshot.memory_usage_bytes);

    println!(
        "Memory tracking test - Current: {} MB, Peak: {} MB",
        final_snapshot.memory_usage_bytes / (1024 * 1024),
        final_snapshot.peak_memory_bytes / (1024 * 1024)
    );

    Ok(())
}

#[tokio::test]
async fn test_indexing_with_simulated_contention() -> Result<()> {
    let workspace = ConcurrentTestWorkspace::new().await?;
    workspace.create_files(50).await?;

    let language_detector = Arc::new(LanguageDetector::new());
    let config = ManagerConfig {
        max_workers: 6, // High concurrency
        memory_budget_bytes: 64 * 1024 * 1024,
        memory_pressure_threshold: 0.7,
        max_queue_size: 100,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 1024 * 1024,
        enabled_languages: vec![],
        incremental_mode: false,
        discovery_batch_size: 5, // Small batches for more contention
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
    let manager = Arc::new(IndexingManager::new(
        config,
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    ));
    let contention_operations = Arc::new(AtomicUsize::new(0));

    // Start indexing
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Spawn tasks that create contention by rapidly accessing manager state
    let mut contention_handles = Vec::new();

    for task_id in 0..10 {
        let manager_clone = Arc::clone(&manager);
        let contention_operations_clone = Arc::clone(&contention_operations);

        let handle = tokio::spawn(async move {
            let mut operations = 0;

            while operations < 200 {
                // Rapidly access different manager operations
                match operations % 6 {
                    0 => {
                        let _ = manager_clone.get_progress().await;
                    }
                    1 => {
                        let _ = manager_clone.get_queue_snapshot().await;
                    }
                    2 => {
                        let _ = manager_clone.get_worker_stats().await;
                    }
                    3 => {
                        let _ = manager_clone.get_status().await;
                    }
                    4 => {
                        let _ = manager_clone.is_memory_pressure();
                    }
                    5 => {
                        // Occasionally try pause/resume to create state contention
                        if operations == 100 && task_id == 0 {
                            let _ = manager_clone.pause_indexing().await;
                            sleep(Duration::from_millis(50)).await;
                            let _ = manager_clone.resume_indexing().await;
                        }
                    }
                    _ => unreachable!(),
                }

                operations += 1;
                contention_operations_clone.fetch_add(1, Ordering::Relaxed);

                // Tight loop to create maximum contention
                if operations % 50 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });

        contention_handles.push(handle);
    }

    // Wait for indexing to complete while contention is happening
    let start_time = Instant::now();
    while start_time.elapsed() < Duration::from_secs(15) {
        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    // Wait for contention tasks
    for handle in contention_handles {
        handle.await?;
    }

    let final_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    let total_contention_ops = contention_operations.load(Ordering::Relaxed);

    println!(
        "Contention test: {} files processed, {} contention operations",
        final_progress.processed_files, total_contention_ops
    );

    // Should complete successfully despite heavy contention
    assert!(final_progress.processed_files > 0);
    assert!(total_contention_ops > 1000);

    Ok(())
}

#[tokio::test]
async fn test_queue_remove_matching_thread_safety() -> Result<()> {
    const NUM_TASKS: usize = 8;
    const ITEMS_PER_TASK: usize = 100;

    let queue = Arc::new(IndexingQueue::unlimited());
    let removed_count = Arc::new(AtomicUsize::new(0));

    // First, populate the queue with items
    for task_id in 0..NUM_TASKS {
        for item_id in 0..ITEMS_PER_TASK {
            let path = if item_id % 3 == 0 {
                format!("/remove/task_{task_id}/special_{item_id}.rs")
            } else {
                format!("/keep/task_{task_id}/normal_{item_id}.rs")
            };

            let item = QueueItem::medium_priority(PathBuf::from(path));
            queue.enqueue(item).await?;
        }
    }

    let initial_count = queue.len();
    println!("Initial queue size: {initial_count}");

    let mut handles = Vec::new();

    // Spawn tasks that concurrently remove items matching different patterns
    for task_id in 0..NUM_TASKS {
        let queue_clone = Arc::clone(&queue);
        let removed_count_clone = Arc::clone(&removed_count);

        let handle = tokio::spawn(async move {
            // Different removal patterns for different tasks
            let pattern = match task_id % 3 {
                0 => "special",     // Remove items with "special" in name
                1 => "/remove/",    // Remove items in "remove" directory
                _ => "nonexistent", // Pattern that matches nothing
            };

            let removed = queue_clone
                .remove_matching(|item| item.file_path.to_string_lossy().contains(pattern))
                .await;

            removed_count_clone.fetch_add(removed, Ordering::Relaxed);
            println!("Task {task_id} removed {removed} items matching '{pattern}'");
        });

        handles.push(handle);
    }

    // Wait for all removal operations
    for handle in handles {
        handle.await?;
    }

    let final_count = queue.len();
    let total_removed = removed_count.load(Ordering::Relaxed);

    println!("Final queue size: {final_count}, total removed: {total_removed}");

    // Verify consistency
    assert_eq!(initial_count, final_count + total_removed);

    // Verify remaining items don't match removal patterns
    let remaining_items = {
        let mut items = Vec::new();
        while let Some(item) = queue.dequeue().await {
            items.push(item);
        }
        items
    };

    for item in remaining_items {
        let path_str = item.file_path.to_string_lossy();
        // Should not contain "special" and should not be in "/remove/" directory
        assert!(!path_str.contains("special"));
        assert!(!path_str.contains("/remove/"));
    }

    Ok(())
}
