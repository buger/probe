//! Integration tests for file watcher and incremental indexing
//!
//! These tests verify that the file watcher correctly detects changes
//! and triggers appropriate indexing updates in the background.

use anyhow::Result;
use lsp_daemon::cache_types::LspOperation;
use lsp_daemon::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
use lsp_daemon::file_watcher::{FileEventType, FileWatcher, FileWatcherConfig};
use lsp_daemon::indexing::{IndexingManager, ManagerConfig};
use lsp_daemon::lsp_cache::{LspCache, LspCacheConfig};
use lsp_daemon::lsp_registry::LspRegistry;
use lsp_daemon::server_manager::SingleServerManager;
use lsp_daemon::LanguageDetector;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::fs;
use tokio::time::{sleep, timeout};

/// Helper struct for managing test workspaces
struct TestWorkspace {
    #[allow(dead_code)]
    temp_dir: TempDir, // Keeps the temp directory alive for the lifetime of the struct
    root_path: PathBuf,
}

impl TestWorkspace {
    async fn new() -> Result<Self> {
        let temp_dir = tempfile::tempdir()?;
        let root_path = temp_dir.path().to_path_buf();

        // Create basic directory structure
        fs::create_dir_all(root_path.join("src")).await?;
        fs::create_dir_all(root_path.join("tests")).await?;

        Ok(Self {
            temp_dir,
            root_path,
        })
    }

    fn path(&self) -> &Path {
        &self.root_path
    }

    async fn create_file(&self, relative_path: &str, content: &str) -> Result<PathBuf> {
        let file_path = self.root_path.join(relative_path);

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(&file_path, content).await?;
        Ok(file_path)
    }

    async fn modify_file(&self, relative_path: &str, content: &str) -> Result<()> {
        let file_path = self.root_path.join(relative_path);
        // Small delay to ensure different mtime
        sleep(Duration::from_millis(10)).await;
        fs::write(file_path, content).await?;
        Ok(())
    }

    async fn delete_file(&self, relative_path: &str) -> Result<()> {
        let file_path = self.root_path.join(relative_path);
        fs::remove_file(file_path).await?;
        Ok(())
    }
}

/// Create a test file watcher configuration
fn create_test_watcher_config() -> FileWatcherConfig {
    FileWatcherConfig {
        poll_interval_secs: 1,     // Fast polling for tests
        event_batch_size: 1,       // Send events immediately
        debounce_interval_ms: 100, // Short debounce for tests
        debug_logging: true,
        max_files_per_workspace: 1000,
        exclude_patterns: vec![
            "*.tmp".to_string(),
            "*/target/*".to_string(),
            "*/.git/*".to_string(),
        ],
        include_patterns: vec![],
        max_file_size_bytes: 1024 * 1024, // 1MB limit
    }
}

#[tokio::test]
async fn test_file_watcher_basic_functionality() -> Result<()> {
    let workspace = TestWorkspace::new().await?;
    let config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(config);

    // Add workspace
    watcher.add_workspace(workspace.path())?;
    let mut receiver = watcher.take_receiver().unwrap();

    // Start watching
    watcher.start()?;

    // Create a file
    workspace.create_file("src/main.rs", "fn main() {}").await?;

    // Wait for create event
    let events = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for create event"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for create event"))?;

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, FileEventType::Created);
    assert!(events[0].file_path.to_string_lossy().contains("main.rs"));

    // Modify the file
    workspace
        .modify_file("src/main.rs", "fn main() { println!(\"hello\"); }")
        .await?;

    // Wait for modify event
    let events = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for modify event"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for modify event"))?;

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, FileEventType::Modified);

    // Delete the file
    workspace.delete_file("src/main.rs").await?;

    // Wait for delete event
    let events = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for delete event"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for delete event"))?;

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, FileEventType::Deleted);

    watcher.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_file_watcher_exclusion_patterns() -> Result<()> {
    let workspace = TestWorkspace::new().await?;
    let config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(config);

    watcher.add_workspace(workspace.path())?;
    let mut receiver = watcher.take_receiver().unwrap();

    watcher.start()?;

    // Create files that should be excluded
    workspace.create_file("temp.tmp", "temporary").await?;
    workspace.create_file("target/debug/app", "binary").await?;
    workspace.create_file(".git/config", "git config").await?;

    // Create file that should be included
    workspace
        .create_file("src/lib.rs", "pub fn hello() {}")
        .await?;

    // Wait for events - should only get the lib.rs file
    let events = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for events"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for events"))?;

    assert_eq!(events.len(), 1);
    assert!(events[0].file_path.to_string_lossy().contains("lib.rs"));

    watcher.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_file_watcher_batch_events() -> Result<()> {
    let workspace = TestWorkspace::new().await?;
    let mut config = create_test_watcher_config();
    config.event_batch_size = 3; // Batch events together
    config.debounce_interval_ms = 500; // Longer debounce

    let mut watcher = FileWatcher::new(config);
    watcher.add_workspace(workspace.path())?;
    let mut receiver = watcher.take_receiver().unwrap();

    watcher.start()?;

    // Create multiple files quickly
    workspace.create_file("src/file1.rs", "// File 1").await?;
    workspace.create_file("src/file2.rs", "// File 2").await?;
    workspace.create_file("src/file3.rs", "// File 3").await?;

    // Wait for batched events
    let events = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for batched events"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for batched events"))?;

    assert_eq!(events.len(), 3);
    for event in events {
        assert_eq!(event.event_type, FileEventType::Created);
        assert!(event.file_path.to_string_lossy().contains(".rs"));
    }

    watcher.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_file_watcher_multiple_workspaces() -> Result<()> {
    let workspace1 = TestWorkspace::new().await?;
    let workspace2 = TestWorkspace::new().await?;

    let config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(config);

    watcher.add_workspace(workspace1.path())?;
    watcher.add_workspace(workspace2.path())?;

    let mut receiver = watcher.take_receiver().unwrap();
    watcher.start()?;

    // Create files in both workspaces
    workspace1.create_file("file1.rs", "// Workspace 1").await?;
    workspace2.create_file("file2.rs", "// Workspace 2").await?;

    // Wait for events from both workspaces
    let events1 = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for workspace 1 events"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for workspace 1 events"))?;

    let events2 = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for workspace 2 events"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for workspace 2 events"))?;

    assert_eq!(events1.len(), 1);
    assert_eq!(events2.len(), 1);

    // Verify events are from correct workspaces
    assert!(
        events1[0].workspace_root == workspace1.path()
            || events1[0].workspace_root == workspace2.path()
    );
    assert!(
        events2[0].workspace_root == workspace1.path()
            || events2[0].workspace_root == workspace2.path()
    );

    // Should be from different workspaces
    assert_ne!(events1[0].workspace_root, events2[0].workspace_root);

    watcher.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_file_watcher_large_file_exclusion() -> Result<()> {
    let workspace = TestWorkspace::new().await?;
    let mut config = create_test_watcher_config();
    config.max_file_size_bytes = 100; // Very small limit

    let mut watcher = FileWatcher::new(config);
    watcher.add_workspace(workspace.path())?;
    let mut receiver = watcher.take_receiver().unwrap();

    watcher.start()?;

    // Create small file (should be detected)
    workspace.create_file("small.rs", "// Small").await?;

    // Create large file (should be ignored)
    let large_content = "// ".repeat(100); // Exceeds 100 byte limit
    workspace.create_file("large.rs", &large_content).await?;

    // Wait for events - should only get the small file
    let events = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for events"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for events"))?;

    assert_eq!(events.len(), 1);
    assert!(events[0].file_path.to_string_lossy().contains("small.rs"));

    watcher.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_incremental_indexing_with_file_watcher() -> Result<()> {
    let workspace = TestWorkspace::new().await?;

    // Create initial files
    workspace
        .create_file(
            "src/main.rs",
            r#"
fn main() {
    println!("Hello, world!");
}

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
        )
        .await?;

    workspace
        .create_file(
            "src/lib.rs",
            r#"
pub mod utils;

pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}
"#,
        )
        .await?;

    // Setup indexing manager with incremental mode
    let manager_config = ManagerConfig {
        incremental_mode: true,
        max_workers: 2,
        memory_budget_bytes: 64 * 1024 * 1024,
        memory_pressure_threshold: 0.8,
        max_queue_size: 100,
        exclude_patterns: vec![],
        include_patterns: vec![],
        max_file_size_bytes: 1024 * 1024,
        enabled_languages: vec!["Rust".to_string()],
        discovery_batch_size: 10,
        status_update_interval_secs: 1,
    };

    let language_detector = Arc::new(LanguageDetector::new());
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
        manager_config.clone(),
        language_detector,
        server_manager,
        call_graph_cache,
        definition_cache,
    );

    // First indexing run
    manager
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Wait for initial indexing to complete
    let start_time = Instant::now();
    while start_time.elapsed() < Duration::from_secs(10) {
        let progress = manager.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    let initial_progress = manager.get_progress().await;
    manager.stop_indexing().await?;

    assert!(initial_progress.processed_files >= 2);
    assert!(initial_progress.is_complete());

    // Setup file watcher
    let watcher_config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(watcher_config);
    watcher.add_workspace(workspace.path())?;
    let mut receiver = watcher.take_receiver().unwrap();
    watcher.start()?;

    // Modify existing file
    workspace
        .modify_file(
            "src/main.rs",
            r#"
fn main() {
    println!("Hello, modified world!");
}

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn subtract(a: i32, b: i32) -> i32 {
    a - b
}
"#,
        )
        .await?;

    // Wait for file change event
    let events = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for file modification event"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for file modification event"))?;

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, FileEventType::Modified);
    assert!(events[0].file_path.to_string_lossy().contains("main.rs"));

    // Create new file
    workspace
        .create_file(
            "src/utils.rs",
            r#"
pub fn is_even(n: i32) -> bool {
    n % 2 == 0
}

pub fn factorial(n: u32) -> u32 {
    match n {
        0 | 1 => 1,
        _ => n * factorial(n - 1),
    }
}
"#,
        )
        .await?;

    // Wait for file creation event
    let events = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for file creation event"))?
        .ok_or_else(|| anyhow::anyhow!("Channel closed waiting for file creation event"))?;

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, FileEventType::Created);
    assert!(events[0].file_path.to_string_lossy().contains("utils.rs"));

    watcher.stop().await?;

    // Second indexing run (incremental) - should only process changed files
    let registry2 = Arc::new(LspRegistry::new().expect("Failed to create registry"));
    let child_processes2 = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let server_manager2 = Arc::new(SingleServerManager::new_with_tracker(
        registry2,
        child_processes2,
    ));

    let cache_config2 = CallGraphCacheConfig {
        capacity: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        invalidation_depth: 1,
        ..Default::default()
    };
    let call_graph_cache2 = Arc::new(CallGraphCache::new(cache_config2));

    let lsp_cache_config2 = LspCacheConfig {
        capacity_per_operation: 100,
        ttl: Duration::from_secs(300),
        eviction_check_interval: Duration::from_secs(30),
        persistent: false,
        cache_directory: None,
    };
    let definition_cache2 = Arc::new(
        LspCache::new(LspOperation::Definition, lsp_cache_config2)
            .expect("Failed to create definition cache"),
    );
    let manager2 = IndexingManager::new(
        manager_config.clone(),
        Arc::new(LanguageDetector::new()),
        server_manager2,
        call_graph_cache2,
        definition_cache2,
    );
    manager2
        .start_indexing(workspace.path().to_path_buf())
        .await?;

    // Wait for incremental indexing
    let start_time = Instant::now();
    while start_time.elapsed() < Duration::from_secs(10) {
        let progress = manager2.get_progress().await;
        if progress.is_complete() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    let incremental_progress = manager2.get_progress().await;
    manager2.stop_indexing().await?;

    // Incremental indexing should detect the changes
    // (exact numbers depend on implementation, but should be > 0)
    assert!(incremental_progress.processed_files > 0);
    assert!(incremental_progress.is_complete());

    println!(
        "Initial indexing: {} files, Incremental: {} files",
        initial_progress.processed_files, incremental_progress.processed_files
    );

    Ok(())
}

#[tokio::test]
async fn test_file_watcher_statistics() -> Result<()> {
    let workspace1 = TestWorkspace::new().await?;
    let workspace2 = TestWorkspace::new().await?;

    let config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(config);

    // Initially no workspaces
    let stats = watcher.get_stats();
    assert_eq!(stats.workspace_count, 0);
    assert_eq!(stats.total_files_tracked, 0);
    assert!(!stats.is_running);

    // Add workspaces
    watcher.add_workspace(workspace1.path())?;
    watcher.add_workspace(workspace2.path())?;

    let stats = watcher.get_stats();
    assert_eq!(stats.workspace_count, 2);
    assert!(!stats.is_running);

    // Start watching
    watcher.start()?;

    let stats = watcher.get_stats();
    assert_eq!(stats.workspace_count, 2);
    assert!(stats.is_running);
    assert_eq!(stats.poll_interval_secs, 1);

    watcher.stop().await?;

    Ok(())
}

#[tokio::test]
async fn test_file_watcher_workspace_management() -> Result<()> {
    let workspace1 = TestWorkspace::new().await?;
    let workspace2 = TestWorkspace::new().await?;

    let config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(config);

    // Add first workspace
    watcher.add_workspace(workspace1.path())?;
    assert_eq!(watcher.get_stats().workspace_count, 1);

    // Add second workspace
    watcher.add_workspace(workspace2.path())?;
    assert_eq!(watcher.get_stats().workspace_count, 2);

    // Remove first workspace
    watcher.remove_workspace(workspace1.path())?;
    assert_eq!(watcher.get_stats().workspace_count, 1);

    // Try to remove non-existent workspace
    let result = watcher.remove_workspace("/nonexistent/path");
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_file_watcher_error_handling() -> Result<()> {
    let config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(config);

    // Try to start watcher with no workspaces
    let result = watcher.start();
    assert!(result.is_err());

    // Add invalid workspace
    let result = watcher.add_workspace("/invalid/nonexistent/path");
    assert!(result.is_err());

    // Add file as workspace (should fail)
    let temp_file = tempfile::NamedTempFile::new()?;
    let result = watcher.add_workspace(temp_file.path());
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_file_watcher_concurrent_operations() -> Result<()> {
    let workspace = TestWorkspace::new().await?;
    let config = create_test_watcher_config();
    let mut watcher = FileWatcher::new(config);

    watcher.add_workspace(workspace.path())?;
    let mut receiver = watcher.take_receiver().unwrap();
    watcher.start()?;

    // Create multiple files concurrently
    let mut handles = Vec::new();

    for i in 0..10 {
        let workspace_path = workspace.path().to_path_buf();
        let handle = tokio::spawn(async move {
            let file_name = format!("concurrent_{i}.rs");
            let content = format!("// Concurrent file {i}");
            let file_path = workspace_path.join(file_name);

            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(&file_path, content).await?;

            Ok::<PathBuf, anyhow::Error>(file_path)
        });
        handles.push(handle);
    }

    // Wait for all file creations
    for handle in handles {
        handle.await??;
    }

    // Collect events - should get all 10 files
    let mut total_events = 0;
    let start_time = Instant::now();

    while start_time.elapsed() < Duration::from_secs(10) && total_events < 10 {
        match timeout(Duration::from_secs(2), receiver.recv()).await {
            Ok(Some(events)) => {
                total_events += events.len();
                for event in events {
                    assert_eq!(event.event_type, FileEventType::Created);
                    assert!(event.file_path.to_string_lossy().contains("concurrent_"));
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    assert_eq!(total_events, 10);

    watcher.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_file_watcher_rapid_changes() -> Result<()> {
    let workspace = TestWorkspace::new().await?;
    let mut config = create_test_watcher_config();
    config.debounce_interval_ms = 200; // Debounce rapid changes

    let mut watcher = FileWatcher::new(config);
    watcher.add_workspace(workspace.path())?;
    let mut receiver = watcher.take_receiver().unwrap();
    watcher.start()?;

    let test_file = "rapid_changes.rs";

    // Create initial file
    workspace.create_file(test_file, "// Initial").await?;

    // Make rapid modifications
    for i in 1..=5 {
        sleep(Duration::from_millis(50)).await;
        workspace
            .modify_file(test_file, &format!("// Modification {i}"))
            .await?;
    }

    // Wait for events - debouncing should reduce the number of events
    let mut all_events = Vec::new();
    let start_time = Instant::now();

    while start_time.elapsed() < Duration::from_secs(5) {
        match timeout(Duration::from_millis(500), receiver.recv()).await {
            Ok(Some(mut events)) => {
                all_events.append(&mut events);
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    // Should have at least create event and some modify events
    // (exact count depends on debouncing implementation)
    assert!(!all_events.is_empty());
    assert!(all_events.len() <= 6); // Should be less than total operations due to debouncing

    let create_events = all_events
        .iter()
        .filter(|e| e.event_type == FileEventType::Created)
        .count();
    let modify_events = all_events
        .iter()
        .filter(|e| e.event_type == FileEventType::Modified)
        .count();

    assert_eq!(create_events, 1);
    assert!(modify_events >= 1);

    watcher.stop().await?;
    Ok(())
}
