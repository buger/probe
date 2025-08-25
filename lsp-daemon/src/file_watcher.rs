//! File watcher for monitoring workspace changes and triggering incremental re-indexing
//!
//! This module provides a polling-based file watcher that monitors multiple workspace
//! directories for changes (creations, modifications, deletions) and emits events
//! through channels for async processing by the indexing system.
//!
//! Key features:
//! - Polling-based approach for maximum portability (no external deps)
//! - Multi-workspace monitoring with configurable patterns
//! - Efficient modification time tracking
//! - Common directory skipping (.git, node_modules, target, etc.)
//! - Configurable poll intervals and batch sizes
//! - Graceful shutdown and error handling

use anyhow::{anyhow, Result};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, trace, warn};

/// Configuration for the file watcher
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWatcherConfig {
    /// Poll interval for checking file changes (seconds)
    pub poll_interval_secs: u64,

    /// Maximum number of files to track per workspace
    pub max_files_per_workspace: usize,

    /// File patterns to exclude from watching
    pub exclude_patterns: Vec<String>,

    /// File patterns to include (empty = include all)
    pub include_patterns: Vec<String>,

    /// Maximum file size to monitor (bytes)
    pub max_file_size_bytes: u64,

    /// Batch size for processing file events
    pub event_batch_size: usize,

    /// Debounce interval to avoid rapid-fire events (milliseconds)
    pub debounce_interval_ms: u64,

    /// Enable detailed logging for debugging
    pub debug_logging: bool,
}

impl Default for FileWatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 2,           // Poll every 2 seconds
            max_files_per_workspace: 50_000, // 50k files max per workspace
            exclude_patterns: vec![
                // Version control
                "*/.git/*".to_string(),
                "*/.svn/*".to_string(),
                "*/.hg/*".to_string(),
                // Build artifacts and dependencies
                "*/node_modules/*".to_string(),
                "*/target/*".to_string(),
                "*/build/*".to_string(),
                "*/dist/*".to_string(),
                "*/.next/*".to_string(),
                "*/__pycache__/*".to_string(),
                "*/venv/*".to_string(),
                "*/env/*".to_string(),
                // IDE and editor files
                "*/.vscode/*".to_string(),
                "*/.idea/*".to_string(),
                "*/.DS_Store".to_string(),
                "*/Thumbs.db".to_string(),
                // Temporary and log files
                "*.tmp".to_string(),
                "*.temp".to_string(),
                "*.log".to_string(),
                "*.swp".to_string(),
                "*~".to_string(),
                // Lock files
                "*.lock".to_string(),
                "Cargo.lock".to_string(),
                "package-lock.json".to_string(),
                "yarn.lock".to_string(),
            ],
            include_patterns: vec![],              // Empty = include all
            max_file_size_bytes: 10 * 1024 * 1024, // 10MB max
            event_batch_size: 100,
            debounce_interval_ms: 500, // 500ms debounce
            debug_logging: false,
        }
    }
}

/// Type of file system event detected
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileEventType {
    /// File was created
    Created,
    /// File was modified (content or metadata changed)
    Modified,
    /// File was deleted
    Deleted,
}

/// File system event containing change information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    /// Path to the file that changed
    pub file_path: PathBuf,
    /// Type of change that occurred
    pub event_type: FileEventType,
    /// Workspace root this file belongs to
    pub workspace_root: PathBuf,
    /// Timestamp when the event was detected
    pub timestamp: u64,
    /// File size at time of event (if available)
    pub file_size: Option<u64>,
}

impl FileEvent {
    fn new(file_path: PathBuf, event_type: FileEventType, workspace_root: PathBuf) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            file_path,
            event_type,
            workspace_root,
            timestamp,
            file_size: None,
        }
    }

    fn with_size(mut self, size: u64) -> Self {
        self.file_size = Some(size);
        self
    }
}

/// Tracks the state of files being monitored
#[derive(Debug)]
struct FileTracker {
    /// Map from file path to (modification_time, file_size)
    files: HashMap<PathBuf, (u64, u64)>,
    /// Workspace root this tracker monitors
    workspace_root: PathBuf,
    /// Configuration
    config: FileWatcherConfig,
}

impl FileTracker {
    fn new(workspace_root: PathBuf, config: FileWatcherConfig) -> Self {
        Self {
            files: HashMap::new(),
            workspace_root,
            config,
        }
    }

    /// Scan workspace and detect changes since last scan
    async fn scan_for_changes(&mut self) -> Result<Vec<FileEvent>> {
        let mut events = Vec::new();
        let mut new_files = HashMap::new();

        if self.config.debug_logging {
            debug!(
                "Scanning workspace {:?} for changes (tracking {} files)",
                self.workspace_root,
                self.files.len()
            );
        }

        // Walk the workspace directory safely using ignore::WalkBuilder
        let mut builder = WalkBuilder::new(&self.workspace_root);

        // CRITICAL: Never follow symlinks to avoid junction point cycles on Windows
        builder.follow_links(false);

        // Stay on the same file system to avoid traversing mount points
        builder.same_file_system(true);

        // CRITICAL: Disable parent directory discovery to prevent climbing into junction cycles
        builder.parents(false);

        // For file watching, we typically want to respect gitignore
        builder.git_ignore(true);
        builder.git_global(false); // Skip global gitignore for performance
        builder.git_exclude(false); // Skip .git/info/exclude for performance

        // Use single thread for file watcher to avoid overwhelming the system
        builder.threads(1);

        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(err) => {
                    if self.config.debug_logging {
                        trace!("Error accessing directory entry: {}", err);
                    }
                    continue;
                }
            };

            // Skip directories
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }

            // Extra defensive check: skip symlinks even though we configured the walker not to follow them
            if entry.file_type().is_some_and(|ft| ft.is_symlink()) {
                if self.config.debug_logging {
                    trace!("Skipping symlink file: {:?}", entry.path());
                }
                continue;
            }

            let file_path = entry.path().to_path_buf();

            // Apply directory exclusion patterns
            if self.should_exclude_path(&file_path) {
                continue;
            }

            // Apply inclusion/exclusion patterns at the file level too
            if self.should_exclude_file(&file_path) {
                continue;
            }

            // Get file metadata
            let metadata = match entry.metadata() {
                Ok(meta) => meta,
                Err(err) => {
                    if self.config.debug_logging {
                        trace!("Failed to get metadata for {:?}: {}", file_path, err);
                    }
                    continue;
                }
            };

            // Check file size limit
            let file_size = metadata.len();
            if file_size > self.config.max_file_size_bytes {
                if self.config.debug_logging {
                    trace!(
                        "Skipping large file: {:?} ({} bytes > {} limit)",
                        file_path,
                        file_size,
                        self.config.max_file_size_bytes
                    );
                }
                continue;
            }

            // Get modification time
            let modified_time = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            // Check for changes
            match self.files.get(&file_path) {
                Some((old_mtime, old_size)) => {
                    // File exists in our tracking - check for modifications
                    if modified_time > *old_mtime || file_size != *old_size {
                        events.push(
                            FileEvent::new(
                                file_path.clone(),
                                FileEventType::Modified,
                                self.workspace_root.clone(),
                            )
                            .with_size(file_size),
                        );

                        if self.config.debug_logging {
                            debug!(
                                "Modified: {:?} (mtime: {} -> {}, size: {} -> {})",
                                file_path, old_mtime, modified_time, old_size, file_size
                            );
                        }
                    }
                }
                None => {
                    // New file
                    events.push(
                        FileEvent::new(
                            file_path.clone(),
                            FileEventType::Created,
                            self.workspace_root.clone(),
                        )
                        .with_size(file_size),
                    );

                    if self.config.debug_logging {
                        debug!("Created: {:?} (size: {})", file_path, file_size);
                    }
                }
            }

            new_files.insert(file_path, (modified_time, file_size));

            // Check if we're exceeding the file limit
            if new_files.len() > self.config.max_files_per_workspace {
                warn!(
                    "Workspace {:?} has too many files ({} > {}), stopping scan",
                    self.workspace_root,
                    new_files.len(),
                    self.config.max_files_per_workspace
                );
                break;
            }
        }

        // Detect deleted files
        for old_path in self.files.keys() {
            if !new_files.contains_key(old_path) {
                events.push(FileEvent::new(
                    old_path.clone(),
                    FileEventType::Deleted,
                    self.workspace_root.clone(),
                ));

                if self.config.debug_logging {
                    debug!("Deleted: {:?}", old_path);
                }
            }
        }

        // Update our tracking
        self.files = new_files;

        if self.config.debug_logging && !events.is_empty() {
            debug!(
                "Detected {} changes in workspace {:?}",
                events.len(),
                self.workspace_root
            );
        }

        Ok(events)
    }

    /// Check if a path should be excluded based on exclude patterns
    fn should_exclude_path(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.config.exclude_patterns {
            if self.matches_pattern(&path_str, pattern) {
                return true;
            }
        }

        false
    }

    /// Check if a file should be excluded
    fn should_exclude_file(&self, file_path: &Path) -> bool {
        // First check exclusion patterns
        if self.should_exclude_path(file_path) {
            return true;
        }

        // If include patterns are specified, file must match at least one
        if !self.config.include_patterns.is_empty() {
            let path_str = file_path.to_string_lossy();
            let mut matches_include = false;

            for pattern in &self.config.include_patterns {
                if self.matches_pattern(&path_str, pattern) {
                    matches_include = true;
                    break;
                }
            }

            if !matches_include {
                return true;
            }
        }

        false
    }

    /// Simple pattern matching with wildcards
    fn matches_pattern(&self, text: &str, pattern: &str) -> bool {
        if pattern.contains('*') {
            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() == 2 {
                let (prefix, suffix) = (parts[0], parts[1]);
                return text.starts_with(prefix) && text.ends_with(suffix);
            } else if parts.len() > 2 {
                // Multiple wildcards - check if text contains all parts in order
                let mut search_start = 0;
                for (i, part) in parts.iter().enumerate() {
                    if part.is_empty() {
                        continue; // Skip empty parts from consecutive '*'
                    }

                    if i == 0 {
                        // First part should be at the beginning
                        if !text.starts_with(part) {
                            return false;
                        }
                        search_start = part.len();
                    } else if i == parts.len() - 1 {
                        // Last part should be at the end
                        return text.ends_with(part);
                    } else {
                        // Middle parts should be found in order
                        if let Some(pos) = text[search_start..].find(part) {
                            search_start += pos + part.len();
                        } else {
                            return false;
                        }
                    }
                }
                return true;
            }
        }

        text.contains(pattern)
    }
}

/// File watcher that monitors multiple workspaces for changes
pub struct FileWatcher {
    /// Configuration
    config: FileWatcherConfig,
    /// File trackers for each workspace
    trackers: HashMap<PathBuf, FileTracker>,
    /// Event sender channel
    event_sender: mpsc::UnboundedSender<Vec<FileEvent>>,
    /// Event receiver channel
    event_receiver: Option<mpsc::UnboundedReceiver<Vec<FileEvent>>>,
    /// Shutdown signal
    shutdown: Arc<AtomicBool>,
    /// Background task handle
    watch_task: Option<tokio::task::JoinHandle<()>>,
}

impl FileWatcher {
    /// Create a new file watcher with the given configuration
    pub fn new(config: FileWatcherConfig) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            config,
            trackers: HashMap::new(),
            event_sender,
            event_receiver: Some(event_receiver),
            shutdown: Arc::new(AtomicBool::new(false)),
            watch_task: None,
        }
    }

    /// Add a workspace to be monitored
    pub fn add_workspace<P: AsRef<Path>>(&mut self, workspace_root: P) -> Result<()> {
        let workspace_root = workspace_root.as_ref().to_path_buf();

        // Canonicalize the path to ensure consistency
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.clone());

        if !canonical_root.exists() {
            return Err(anyhow!(
                "Workspace root does not exist: {:?}",
                canonical_root
            ));
        }

        if !canonical_root.is_dir() {
            return Err(anyhow!(
                "Workspace root is not a directory: {:?}",
                canonical_root
            ));
        }

        info!("Adding workspace for file watching: {:?}", canonical_root);

        let tracker = FileTracker::new(canonical_root.clone(), self.config.clone());
        self.trackers.insert(canonical_root, tracker);

        Ok(())
    }

    /// Remove a workspace from monitoring
    pub fn remove_workspace<P: AsRef<Path>>(&mut self, workspace_root: P) -> Result<()> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let canonical_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.clone());

        if self.trackers.remove(&canonical_root).is_some() {
            info!("Removed workspace from file watching: {:?}", canonical_root);
            Ok(())
        } else {
            Err(anyhow!(
                "Workspace not found for removal: {:?}",
                canonical_root
            ))
        }
    }

    /// Start the file watcher background task
    pub fn start(&mut self) -> Result<()> {
        if self.watch_task.is_some() {
            return Err(anyhow!("File watcher is already running"));
        }

        if self.trackers.is_empty() {
            return Err(anyhow!("No workspaces configured for watching"));
        }

        info!(
            "Starting file watcher for {} workspaces (poll interval: {}s)",
            self.trackers.len(),
            self.config.poll_interval_secs
        );

        let shutdown = Arc::clone(&self.shutdown);
        let event_sender = self.event_sender.clone();
        let trackers = std::mem::take(&mut self.trackers);
        let config = self.config.clone();

        let task = tokio::spawn(async move {
            Self::watch_loop(config, trackers, event_sender, shutdown).await;
        });

        self.watch_task = Some(task);
        Ok(())
    }

    /// Stop the file watcher
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping file watcher");

        self.shutdown.store(true, Ordering::Relaxed);

        if let Some(task) = self.watch_task.take() {
            // Give the task a moment to shutdown gracefully
            match tokio::time::timeout(Duration::from_secs(5), task).await {
                Ok(result) => {
                    if let Err(e) = result {
                        warn!("File watcher task error during shutdown: {}", e);
                    }
                }
                Err(_) => {
                    warn!("File watcher task did not shutdown within timeout");
                }
            }
        }

        info!("File watcher stopped");
        Ok(())
    }

    /// Get the event receiver channel
    pub fn take_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<Vec<FileEvent>>> {
        self.event_receiver.take()
    }

    /// Get statistics about the file watcher
    pub fn get_stats(&self) -> FileWatcherStats {
        let total_files = self.trackers.values().map(|t| t.files.len()).sum();

        FileWatcherStats {
            workspace_count: self.trackers.len(),
            total_files_tracked: total_files,
            is_running: self.watch_task.is_some() && !self.shutdown.load(Ordering::Relaxed),
            poll_interval_secs: self.config.poll_interval_secs,
        }
    }

    /// Main watching loop that runs in the background
    async fn watch_loop(
        config: FileWatcherConfig,
        mut trackers: HashMap<PathBuf, FileTracker>,
        event_sender: mpsc::UnboundedSender<Vec<FileEvent>>,
        shutdown: Arc<AtomicBool>,
    ) {
        let mut interval_timer = interval(Duration::from_secs(config.poll_interval_secs));
        let mut event_buffer = Vec::new();

        debug!("File watcher loop started");

        while !shutdown.load(Ordering::Relaxed) {
            interval_timer.tick().await;

            if config.debug_logging {
                trace!("File watcher tick - scanning {} workspaces", trackers.len());
            }

            // Scan all workspaces for changes
            for (workspace_root, tracker) in &mut trackers {
                match tracker.scan_for_changes().await {
                    Ok(mut events) => {
                        if !events.is_empty() {
                            event_buffer.append(&mut events);
                        }
                    }
                    Err(e) => {
                        error!(
                            "Error scanning workspace {:?} for changes: {}",
                            workspace_root, e
                        );
                    }
                }

                // Yield control to prevent blocking
                tokio::task::yield_now().await;

                // Check shutdown signal frequently
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
            }

            // Send accumulated events if we have any
            if !event_buffer.is_empty() {
                // Apply debouncing by batching events
                if event_buffer.len() >= config.event_batch_size {
                    let batch = std::mem::take(&mut event_buffer);

                    if config.debug_logging {
                        debug!("Sending batch of {} file events", batch.len());
                    }

                    if event_sender.send(batch).is_err() {
                        error!("Failed to send file events - receiver dropped");
                        break;
                    }
                } else if config.debounce_interval_ms > 0 {
                    // Wait for debounce interval before sending smaller batches
                    sleep(Duration::from_millis(config.debounce_interval_ms)).await;

                    let batch = std::mem::take(&mut event_buffer);
                    if !batch.is_empty() {
                        if config.debug_logging {
                            debug!("Sending debounced batch of {} file events", batch.len());
                        }

                        if event_sender.send(batch).is_err() {
                            error!("Failed to send debounced file events - receiver dropped");
                            break;
                        }
                    }
                }
            }
        }

        // Send any remaining events before shutting down
        if !event_buffer.is_empty() {
            let _ = event_sender.send(event_buffer);
        }

        debug!("File watcher loop terminated");
    }
}

/// Statistics about the file watcher
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWatcherStats {
    pub workspace_count: usize,
    pub total_files_tracked: usize,
    pub is_running: bool,
    pub poll_interval_secs: u64,
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        debug!("FileWatcher dropped - shutdown signal sent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_file_watcher_creation() {
        let config = FileWatcherConfig::default();
        let watcher = FileWatcher::new(config);

        assert_eq!(watcher.trackers.len(), 0);
        assert!(watcher.watch_task.is_none());
    }

    #[tokio::test]
    async fn test_add_workspace() {
        let temp_dir = TempDir::new().unwrap();
        let mut watcher = FileWatcher::new(FileWatcherConfig::default());

        // Add valid workspace
        let result = watcher.add_workspace(temp_dir.path());
        assert!(result.is_ok());
        assert_eq!(watcher.trackers.len(), 1);

        // Try to add non-existent workspace
        let invalid_path = temp_dir.path().join("nonexistent");
        let result = watcher.add_workspace(&invalid_path);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pattern_matching() {
        let config = FileWatcherConfig::default();
        let temp_dir = TempDir::new().unwrap();
        let tracker = FileTracker::new(temp_dir.path().to_path_buf(), config);

        // Test exclusion patterns
        assert!(tracker.matches_pattern("/path/node_modules/file.js", "*/node_modules/*"));
        assert!(tracker.matches_pattern("test.tmp", "*.tmp"));
        assert!(!tracker.matches_pattern("test.rs", "*.tmp"));

        // Test exact matches
        assert!(tracker.matches_pattern("exact_match", "exact"));
        assert!(!tracker.matches_pattern("no_match", "different"));
    }

    #[tokio::test]
    async fn test_file_change_detection() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");

        let config = FileWatcherConfig {
            debug_logging: true,
            ..FileWatcherConfig::default()
        };

        let mut tracker = FileTracker::new(temp_dir.path().to_path_buf(), config);

        // Initial scan - no files
        let events = tracker.scan_for_changes().await.unwrap();
        assert_eq!(events.len(), 0);

        // Create a file
        fs::write(&test_file, "initial content").unwrap();
        let events = tracker.scan_for_changes().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, FileEventType::Created);

        // Modify the file
        tokio::time::sleep(Duration::from_millis(10)).await; // Ensure different mtime
        fs::write(&test_file, "modified content").unwrap();
        let events = tracker.scan_for_changes().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, FileEventType::Modified);

        // Delete the file
        fs::remove_file(&test_file).unwrap();
        let events = tracker.scan_for_changes().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, FileEventType::Deleted);
    }

    #[tokio::test]
    async fn test_exclusion_patterns() {
        let temp_dir = TempDir::new().unwrap();

        // Create some files and directories
        let git_dir = temp_dir.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("config"), "git config").unwrap();

        let node_modules = temp_dir.path().join("node_modules");
        fs::create_dir_all(&node_modules).unwrap();
        fs::write(node_modules.join("package.js"), "module").unwrap();

        let src_file = temp_dir.path().join("src.rs");
        fs::write(&src_file, "fn main() {}").unwrap();

        let config = FileWatcherConfig::default();
        let mut tracker = FileTracker::new(temp_dir.path().to_path_buf(), config);

        let events = tracker.scan_for_changes().await.unwrap();

        // Should only detect src.rs, not the excluded files
        assert_eq!(events.len(), 1);
        assert!(events[0].file_path.ends_with("src.rs"));
    }

    #[tokio::test]
    async fn test_watcher_lifecycle() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.txt");

        let config = FileWatcherConfig {
            poll_interval_secs: 1,
            event_batch_size: 1,     // Send events immediately
            debounce_interval_ms: 0, // No debouncing for test
            debug_logging: true,
            ..FileWatcherConfig::default()
        };

        let mut watcher = FileWatcher::new(config);
        watcher.add_workspace(temp_dir.path()).unwrap();

        let mut receiver = watcher.take_receiver().unwrap();

        // Start the watcher
        watcher.start().unwrap();

        // Create a file and wait for event
        fs::write(&test_file, "content").unwrap();

        let events = timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("Timeout waiting for file event")
            .expect("Channel closed");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, FileEventType::Created);
        assert!(events[0].file_path.ends_with("test.txt"));

        // Stop the watcher
        watcher.stop().await.unwrap();
    }

    #[test]
    fn test_file_watcher_stats() {
        let config = FileWatcherConfig::default();
        let watcher = FileWatcher::new(config);

        let stats = watcher.get_stats();
        assert_eq!(stats.workspace_count, 0);
        assert_eq!(stats.total_files_tracked, 0);
        assert!(!stats.is_running);
        assert_eq!(stats.poll_interval_secs, 2);
    }
}
