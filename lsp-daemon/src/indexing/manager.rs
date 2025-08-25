//! Indexing manager orchestrates all indexing operations
//!
//! This module provides the main IndexingManager that coordinates:
//! - Worker pool management with configurable concurrency
//! - File discovery and enumeration  
//! - Priority assignment and queue management
//! - Memory budget tracking and backpressure handling
//! - Language-specific pipeline execution
//! - Progress reporting and status monitoring

use crate::cache_types::DefinitionInfo;
use crate::call_graph_cache::CallGraphCache;
use crate::indexing::{
    pipelines::SymbolInfo, IndexingConfig, IndexingPipeline, IndexingProgress, IndexingQueue,
    LanguageStrategyFactory, Priority, QueueItem,
};
use crate::language_detector::{Language, LanguageDetector};
use crate::lsp_cache::LspCache;
use crate::server_manager::SingleServerManager;
use anyhow::{anyhow, Result};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};
use tokio::time::{interval, sleep, timeout};
use tracing::{debug, error, info, warn};

/// Configuration for the indexing manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerConfig {
    /// Maximum number of worker threads
    pub max_workers: usize,

    /// Memory budget in bytes (0 = unlimited)
    pub memory_budget_bytes: u64,

    /// Memory usage threshold to trigger backpressure (0.0-1.0)
    pub memory_pressure_threshold: f64,

    /// Maximum queue size (0 = unlimited)
    pub max_queue_size: usize,

    /// File patterns to exclude from indexing
    pub exclude_patterns: Vec<String>,

    /// File patterns to include (empty = include all)
    pub include_patterns: Vec<String>,

    /// Maximum file size to index (bytes)
    pub max_file_size_bytes: u64,

    /// Languages to enable for indexing (empty = all supported)
    pub enabled_languages: Vec<String>,

    /// Whether to use file modification time for incremental indexing
    pub incremental_mode: bool,

    /// Batch size for file discovery
    pub discovery_batch_size: usize,

    /// Interval between status updates (seconds)
    pub status_update_interval_secs: u64,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            max_workers: num_cpus::get().max(2),    // At least 2 workers
            memory_budget_bytes: 512 * 1024 * 1024, // 512MB default
            memory_pressure_threshold: 0.8,         // 80% threshold
            max_queue_size: 10000,                  // 10k files max
            exclude_patterns: vec![
                "*.git/*".to_string(),
                "*/node_modules/*".to_string(),
                "*/target/*".to_string(),
                "*/build/*".to_string(),
                "*/dist/*".to_string(),
                "*.tmp".to_string(),
                "*.log".to_string(),
                "*.lock".to_string(),
            ],
            include_patterns: vec![],              // Empty = include all
            max_file_size_bytes: 10 * 1024 * 1024, // 10MB max per file
            enabled_languages: vec![],             // Empty = all languages
            incremental_mode: true,
            discovery_batch_size: 100,
            status_update_interval_secs: 5,
        }
    }
}

/// Current status of the indexing manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ManagerStatus {
    /// Manager is idle, not currently indexing
    Idle,

    /// Discovering files to index
    Discovering,

    /// Actively indexing files with worker pool
    Indexing,

    /// Indexing paused due to memory pressure or other constraints
    Paused,

    /// Shutting down, stopping workers
    ShuttingDown,

    /// Manager has shut down
    Shutdown,

    /// Error state - indexing failed
    Error(String),
}

/// Statistics for worker performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStats {
    pub worker_id: usize,
    pub files_processed: u64,
    pub bytes_processed: u64,
    pub symbols_extracted: u64,
    pub errors_encountered: u64,
    pub current_file: Option<PathBuf>,
    pub is_active: bool,
    pub last_activity: Option<u64>, // Unix timestamp
}

/// Main indexing manager that orchestrates all indexing operations
pub struct IndexingManager {
    /// Configuration
    config: ManagerConfig,

    /// Current manager status
    status: Arc<RwLock<ManagerStatus>>,

    /// File discovery and processing queue
    queue: Arc<IndexingQueue>,

    /// Progress tracker
    progress: Arc<IndexingProgress>,

    /// Language detection
    language_detector: Arc<LanguageDetector>,

    /// Processing pipelines for each language
    pipelines: Arc<RwLock<HashMap<Language, IndexingPipeline>>>,

    /// Worker pool semaphore
    worker_semaphore: Arc<Semaphore>,

    /// Shutdown signal
    shutdown_signal: Arc<AtomicBool>,

    /// Active worker handles
    worker_handles: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,

    /// Worker statistics
    worker_stats: Arc<RwLock<HashMap<usize, WorkerStats>>>,

    /// Next worker ID for assignment
    next_worker_id: Arc<AtomicUsize>,

    /// Background task handles
    background_tasks: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,

    /// Files already indexed (for incremental mode)
    indexed_files: Arc<RwLock<HashMap<PathBuf, u64>>>, // path -> modification timestamp

    /// Current memory usage tracking
    current_memory_usage: Arc<AtomicU64>,

    /// LSP server manager for language server pool management
    server_manager: Arc<SingleServerManager>,

    /// Call graph cache for caching LSP call hierarchy data
    call_graph_cache: Arc<CallGraphCache>,

    /// Definition cache for caching symbol definitions
    definition_cache: Arc<LspCache<DefinitionInfo>>,

    /// Start time for performance calculations
    #[allow(dead_code)]
    start_time: Instant,
}

impl IndexingManager {
    /// Create a new indexing manager with the specified configuration and LSP dependencies
    pub fn new(
        config: ManagerConfig,
        language_detector: Arc<LanguageDetector>,
        server_manager: Arc<SingleServerManager>,
        call_graph_cache: Arc<CallGraphCache>,
        definition_cache: Arc<LspCache<DefinitionInfo>>,
    ) -> Self {
        let queue = Arc::new(IndexingQueue::new(config.max_queue_size));
        let progress = Arc::new(IndexingProgress::new());
        let worker_semaphore = Arc::new(Semaphore::new(config.max_workers));

        Self {
            config,
            status: Arc::new(RwLock::new(ManagerStatus::Idle)),
            queue,
            progress,
            language_detector,
            pipelines: Arc::new(RwLock::new(HashMap::new())),
            worker_semaphore,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            worker_handles: Arc::new(RwLock::new(Vec::new())),
            worker_stats: Arc::new(RwLock::new(HashMap::new())),
            next_worker_id: Arc::new(AtomicUsize::new(1)),
            background_tasks: Arc::new(RwLock::new(Vec::new())),
            indexed_files: Arc::new(RwLock::new(HashMap::new())),
            current_memory_usage: Arc::new(AtomicU64::new(0)),
            server_manager,
            call_graph_cache,
            definition_cache,
            start_time: Instant::now(),
        }
    }

    /// Create a new indexing manager from the comprehensive IndexingConfig
    pub fn from_indexing_config(
        config: &IndexingConfig,
        language_detector: Arc<LanguageDetector>,
        server_manager: Arc<SingleServerManager>,
        call_graph_cache: Arc<CallGraphCache>,
        definition_cache: Arc<LspCache<DefinitionInfo>>,
    ) -> Self {
        // Convert comprehensive config to legacy ManagerConfig for compatibility
        let manager_config = ManagerConfig {
            max_workers: config.max_workers,
            memory_budget_bytes: config.memory_budget_mb * 1024 * 1024,
            memory_pressure_threshold: config.memory_pressure_threshold,
            max_queue_size: config.max_queue_size,
            exclude_patterns: config.global_exclude_patterns.clone(),
            include_patterns: config.global_include_patterns.clone(),
            max_file_size_bytes: config.max_file_size_bytes,
            enabled_languages: config
                .priority_languages
                .iter()
                .map(|l| format!("{l:?}"))
                .collect(),
            incremental_mode: config.incremental_mode,
            discovery_batch_size: config.discovery_batch_size,
            status_update_interval_secs: config.status_update_interval_secs,
        };

        Self::new(
            manager_config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        )
    }

    /// Start indexing the specified directory
    pub async fn start_indexing(&self, root_path: PathBuf) -> Result<()> {
        // Check if already running
        let current_status = self.status.read().await;
        match *current_status {
            ManagerStatus::Indexing | ManagerStatus::Discovering => {
                return Err(anyhow!("Indexing is already in progress"));
            }
            ManagerStatus::ShuttingDown | ManagerStatus::Shutdown => {
                return Err(anyhow!("Manager is shutting down"));
            }
            _ => {}
        }
        drop(current_status);

        info!("Starting indexing for directory: {:?}", root_path);

        // Reset state
        self.reset_state().await;

        // Update status
        *self.status.write().await = ManagerStatus::Discovering;

        // Start background tasks
        self.start_background_tasks().await?;

        // Start file discovery
        self.start_file_discovery(root_path).await?;

        // Update status
        *self.status.write().await = ManagerStatus::Indexing;

        // Start worker pool
        self.start_worker_pool().await?;

        info!("Indexing started successfully");
        Ok(())
    }

    /// Stop indexing and shutdown all workers
    pub async fn stop_indexing(&self) -> Result<()> {
        info!("Stopping indexing...");

        // Set shutdown signal
        self.shutdown_signal.store(true, Ordering::Relaxed);

        // Update status
        *self.status.write().await = ManagerStatus::ShuttingDown;

        // Pause the queue to prevent new work
        self.queue.pause();

        // Wait for workers to finish with timeout
        self.shutdown_workers().await?;

        // Stop background tasks
        self.shutdown_background_tasks().await;

        // Update status
        *self.status.write().await = ManagerStatus::Shutdown;

        info!("Indexing stopped successfully");
        Ok(())
    }

    /// Pause indexing (can be resumed later)
    pub async fn pause_indexing(&self) -> Result<()> {
        let mut status = self.status.write().await;
        match *status {
            ManagerStatus::Indexing => {
                *status = ManagerStatus::Paused;
                self.queue.pause();
                info!("Indexing paused");
                Ok(())
            }
            _ => Err(anyhow!("Can only pause when indexing is active")),
        }
    }

    /// Resume paused indexing
    pub async fn resume_indexing(&self) -> Result<()> {
        let mut status = self.status.write().await;
        match *status {
            ManagerStatus::Paused => {
                *status = ManagerStatus::Indexing;
                self.queue.resume();
                info!("Indexing resumed");
                Ok(())
            }
            _ => Err(anyhow!("Can only resume when indexing is paused")),
        }
    }

    /// Get current indexing status
    pub async fn get_status(&self) -> ManagerStatus {
        self.status.read().await.clone()
    }

    /// Get progress information
    pub async fn get_progress(&self) -> crate::indexing::ProgressSnapshot {
        self.progress.get_snapshot()
    }

    /// Get queue information
    pub async fn get_queue_snapshot(&self) -> crate::indexing::QueueSnapshot {
        self.queue.get_snapshot().await
    }

    /// Get worker statistics
    pub async fn get_worker_stats(&self) -> Vec<WorkerStats> {
        self.worker_stats.read().await.values().cloned().collect()
    }

    /// Check if memory pressure requires throttling
    pub fn is_memory_pressure(&self) -> bool {
        if self.config.memory_budget_bytes == 0 {
            return false; // No limit
        }

        let current = self.current_memory_usage.load(Ordering::Relaxed);
        let threshold =
            (self.config.memory_budget_bytes as f64 * self.config.memory_pressure_threshold) as u64;

        current > threshold
    }

    /// Reset internal state for new indexing session
    async fn reset_state(&self) {
        self.progress.reset();
        self.queue.clear().await;
        self.shutdown_signal.store(false, Ordering::Relaxed);
        self.current_memory_usage.store(0, Ordering::Relaxed);
        self.worker_stats.write().await.clear();

        // Clear indexed files if not in incremental mode
        if !self.config.incremental_mode {
            self.indexed_files.write().await.clear();
        }
    }

    /// Start background monitoring and maintenance tasks
    async fn start_background_tasks(&self) -> Result<()> {
        let mut tasks = self.background_tasks.write().await;

        // Start status reporting task
        {
            let progress = Arc::clone(&self.progress);
            let queue = Arc::clone(&self.queue);
            let interval_secs = self.config.status_update_interval_secs;
            let shutdown = Arc::clone(&self.shutdown_signal);

            let status_task = tokio::spawn(async move {
                let mut interval = interval(Duration::from_secs(interval_secs));

                while !shutdown.load(Ordering::Relaxed) {
                    interval.tick().await;

                    let progress_snapshot = progress.get_snapshot();
                    let queue_snapshot = queue.get_snapshot().await;

                    debug!("Indexing status - Progress: {}/{} files ({:.1}%), Queue: {} items, Workers: {}",
                        progress_snapshot.processed_files + progress_snapshot.failed_files + progress_snapshot.skipped_files,
                        progress_snapshot.total_files,
                        if progress_snapshot.total_files > 0 {
                            ((progress_snapshot.processed_files + progress_snapshot.failed_files + progress_snapshot.skipped_files) as f64 / progress_snapshot.total_files as f64) * 100.0
                        } else { 0.0 },
                        queue_snapshot.total_items,
                        progress_snapshot.active_workers
                    );
                }

                debug!("Status reporting task shut down");
            });

            tasks.push(status_task);
        }

        // Start memory monitoring task
        {
            let memory_usage = Arc::clone(&self.current_memory_usage);
            let progress = Arc::clone(&self.progress);
            let shutdown = Arc::clone(&self.shutdown_signal);

            let memory_task = tokio::spawn(async move {
                let mut interval = interval(Duration::from_secs(10)); // Check every 10 seconds

                while !shutdown.load(Ordering::Relaxed) {
                    interval.tick().await;

                    let current = memory_usage.load(Ordering::Relaxed);
                    progress.update_memory_usage(current);

                    // Could add memory cleanup logic here if needed
                }

                debug!("Memory monitoring task shut down");
            });

            tasks.push(memory_task);
        }

        info!("Started {} background tasks", tasks.len());
        Ok(())
    }

    /// Shutdown all background tasks
    async fn shutdown_background_tasks(&self) {
        let mut tasks = self.background_tasks.write().await;

        for task in tasks.drain(..) {
            task.abort();
            let _ = task.await; // Ignore errors from aborted tasks
        }

        debug!("Shut down all background tasks");
    }

    /// Start file discovery in the specified directory
    async fn start_file_discovery(&self, root_path: PathBuf) -> Result<()> {
        let queue = Arc::clone(&self.queue);
        let progress = Arc::clone(&self.progress);
        let config = self.config.clone();
        let language_detector = Arc::clone(&self.language_detector);
        let indexed_files = Arc::clone(&self.indexed_files);
        let shutdown = Arc::clone(&self.shutdown_signal);

        // Spawn file discovery task
        let discovery_task = tokio::spawn(async move {
            match Self::discover_files_recursive(
                root_path,
                queue,
                progress,
                config,
                language_detector,
                indexed_files,
                shutdown,
            )
            .await
            {
                Ok(discovered) => {
                    info!("File discovery completed - {} files discovered", discovered);
                }
                Err(e) => {
                    error!("File discovery failed: {}", e);
                }
            }
        });

        // Store the task handle
        self.background_tasks.write().await.push(discovery_task);

        Ok(())
    }

    /// Recursive file discovery implementation
    async fn discover_files_recursive(
        root_path: PathBuf,
        queue: Arc<IndexingQueue>,
        progress: Arc<IndexingProgress>,
        config: ManagerConfig,
        language_detector: Arc<LanguageDetector>,
        indexed_files: Arc<RwLock<HashMap<PathBuf, u64>>>,
        shutdown: Arc<AtomicBool>,
    ) -> Result<u64> {
        let mut discovered_count = 0u64;
        let mut batch = Vec::new();

        // Use ignore::WalkBuilder for safe recursive directory traversal
        let mut builder = WalkBuilder::new(&root_path);

        // CRITICAL: Never follow symlinks to avoid junction point cycles on Windows
        builder.follow_links(false);

        // Stay on the same file system to avoid traversing mount points
        builder.same_file_system(true);

        // CRITICAL: Disable parent directory discovery to prevent climbing into junction cycles
        builder.parents(false);

        // Configure gitignore handling (optional - can be disabled for indexing)
        builder.git_ignore(true);
        builder.git_global(false); // Skip global gitignore for performance
        builder.git_exclude(false); // Skip .git/info/exclude for performance

        // Enable parallel walking for large directories
        builder.threads(1); // Use 1 thread to avoid overwhelming the system during indexing

        for result in builder.build() {
            if shutdown.load(Ordering::Relaxed) {
                debug!("File discovery interrupted by shutdown signal");
                break;
            }

            let entry = match result {
                Ok(entry) => entry,
                Err(e) => {
                    warn!("Error accessing directory entry: {}", e);
                    continue;
                }
            };

            // Skip directories
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }

            // Extra defensive check: skip symlinks even though we configured the walker not to follow them
            if entry.file_type().is_some_and(|ft| ft.is_symlink()) {
                debug!("Skipping symlink file: {:?}", entry.path());
                continue;
            }

            let file_path = entry.path().to_path_buf();

            // Apply exclusion patterns
            if Self::should_exclude_file(&file_path, &config.exclude_patterns) {
                continue;
            }

            // Apply inclusion patterns if specified
            if !config.include_patterns.is_empty()
                && !Self::should_include_file(&file_path, &config.include_patterns)
            {
                continue;
            }

            // Check file size
            if let Ok(metadata) = entry.metadata() {
                if metadata.len() > config.max_file_size_bytes {
                    debug!(
                        "Skipping large file: {:?} ({} bytes)",
                        file_path,
                        metadata.len()
                    );
                    continue;
                }

                // Apply language-specific filtering strategies
                if let Ok(language) = language_detector.detect(&file_path) {
                    if language != Language::Unknown {
                        let strategy = LanguageStrategyFactory::create_strategy(language);

                        // Check if the language strategy says this file should be processed
                        if !strategy.should_process_file(&file_path) {
                            debug!(
                                "Skipping file based on language strategy: {:?} (language: {:?})",
                                file_path, language
                            );
                            continue;
                        }

                        // Check if it's a test file and tests are excluded by the strategy
                        if strategy.is_test_file(&file_path)
                            && !strategy.file_strategy.include_tests
                        {
                            debug!(
                                "Skipping test file: {:?} (language: {:?})",
                                file_path, language
                            );
                            continue;
                        }

                        // Check file size against strategy limits
                        if metadata.len() > strategy.file_strategy.max_file_size {
                            debug!(
                                "Skipping file due to language strategy size limit: {:?} ({} bytes, limit: {} bytes)",
                                file_path, metadata.len(), strategy.file_strategy.max_file_size
                            );
                            continue;
                        }
                    }
                }

                // Check if already indexed (incremental mode)
                if config.incremental_mode {
                    let modified_time = metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    let indexed = indexed_files.read().await;
                    if let Some(&last_indexed) = indexed.get(&file_path) {
                        if modified_time <= last_indexed {
                            continue; // File hasn't changed since last index
                        }
                    }
                }
            }

            // Detect language
            let language = language_detector
                .detect(&file_path)
                .unwrap_or(Language::Unknown);

            // Filter by enabled languages if specified
            if !config.enabled_languages.is_empty() {
                let language_str = language.as_str();
                if !config.enabled_languages.contains(&language_str.to_string()) {
                    continue;
                }
            }

            // Determine priority based on language and file characteristics
            let priority = Self::determine_priority(&file_path, language);

            // Create queue item
            let item = QueueItem::new(file_path, priority)
                .with_language_hint(language.as_str().to_string())
                .with_estimated_size(entry.metadata().ok().map(|m| m.len()).unwrap_or(1024));

            batch.push(item);
            discovered_count += 1;

            // Process batch when it reaches the configured size
            if batch.len() >= config.discovery_batch_size {
                let batch_size = batch.len();
                if let Err(e) = queue.enqueue_batch(batch).await {
                    error!("Failed to enqueue batch: {}", e);
                }
                progress.add_total_files(batch_size as u64);
                batch = Vec::new();

                // Small yield to allow other tasks to run
                tokio::task::yield_now().await;
            }
        }

        // Process remaining batch
        if !batch.is_empty() {
            let batch_size = batch.len();
            if let Err(e) = queue.enqueue_batch(batch).await {
                error!("Failed to enqueue final batch: {}", e);
            }
            progress.add_total_files(batch_size as u64);
        }

        Ok(discovered_count)
    }

    /// Check if file should be excluded based on patterns
    fn should_exclude_file(file_path: &Path, patterns: &[String]) -> bool {
        let path_str = file_path.to_string_lossy();

        for pattern in patterns {
            if Self::matches_pattern(&path_str, pattern) {
                return true;
            }
        }

        false
    }

    /// Check if file should be included based on patterns
    fn should_include_file(file_path: &Path, patterns: &[String]) -> bool {
        let path_str = file_path.to_string_lossy();

        for pattern in patterns {
            if Self::matches_pattern(&path_str, pattern) {
                return true;
            }
        }

        false
    }

    /// Simple pattern matching (supports * wildcards)
    fn matches_pattern(text: &str, pattern: &str) -> bool {
        // Simple glob-like pattern matching
        if pattern.contains('*') {
            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() == 2 {
                let (prefix, suffix) = (parts[0], parts[1]);
                return text.starts_with(prefix) && text.ends_with(suffix);
            } else if parts.len() > 2 {
                // Multiple wildcards - check if text contains all the parts in order
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

    /// Determine indexing priority for a file using language-specific strategies
    fn determine_priority(file_path: &Path, language: Language) -> Priority {
        let strategy = LanguageStrategyFactory::create_strategy(language);
        let language_priority = strategy.calculate_file_priority(file_path);

        // Convert language-specific priority to queue priority
        match language_priority {
            crate::indexing::IndexingPriority::Critical => Priority::Critical,
            crate::indexing::IndexingPriority::High => Priority::High,
            crate::indexing::IndexingPriority::Medium => Priority::Medium,
            crate::indexing::IndexingPriority::Low => Priority::Low,
            crate::indexing::IndexingPriority::Minimal => Priority::Low, // Map minimal to low
        }
    }

    /// Start the worker pool to process queued files
    async fn start_worker_pool(&self) -> Result<()> {
        let mut handles = self.worker_handles.write().await;

        for _ in 0..self.config.max_workers {
            let worker_id = self.next_worker_id.fetch_add(1, Ordering::Relaxed);
            let handle = self.spawn_worker(worker_id).await?;
            handles.push(handle);
        }

        info!("Started worker pool with {} workers", handles.len());
        Ok(())
    }

    /// Spawn a single worker task
    async fn spawn_worker(&self, worker_id: usize) -> Result<tokio::task::JoinHandle<()>> {
        // Initialize worker stats
        {
            let mut stats = self.worker_stats.write().await;
            stats.insert(
                worker_id,
                WorkerStats {
                    worker_id,
                    files_processed: 0,
                    bytes_processed: 0,
                    symbols_extracted: 0,
                    errors_encountered: 0,
                    current_file: None,
                    is_active: false,
                    last_activity: None,
                },
            );
        }

        let queue = Arc::clone(&self.queue);
        let progress = Arc::clone(&self.progress);
        let pipelines = Arc::clone(&self.pipelines);
        let worker_stats = Arc::clone(&self.worker_stats);
        let language_detector = Arc::clone(&self.language_detector);
        let semaphore = Arc::clone(&self.worker_semaphore);
        let shutdown = Arc::clone(&self.shutdown_signal);
        let current_memory = Arc::clone(&self.current_memory_usage);
        let server_manager = Arc::clone(&self.server_manager);
        let call_graph_cache = Arc::clone(&self.call_graph_cache);
        let definition_cache = Arc::clone(&self.definition_cache);
        let config = self.config.clone();

        let handle = tokio::spawn(async move {
            debug!("Worker {} starting", worker_id);
            progress.add_worker();

            while !shutdown.load(Ordering::Relaxed) {
                // Acquire semaphore permit
                let _permit = match timeout(Duration::from_millis(100), semaphore.acquire()).await {
                    Ok(Ok(permit)) => permit,
                    Ok(Err(_)) => {
                        // Semaphore closed, shutdown
                        break;
                    }
                    Err(_) => {
                        // Timeout, check shutdown signal and continue
                        continue;
                    }
                };

                // Check memory pressure
                let memory_usage = current_memory.load(Ordering::Relaxed);
                if config.memory_budget_bytes > 0 && memory_usage > config.memory_budget_bytes {
                    debug!("Worker {} waiting due to memory pressure", worker_id);
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }

                // Get next item from queue
                let item = match queue.dequeue().await {
                    Some(item) => item,
                    None => {
                        // No work available, short sleep
                        sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                };

                // Update worker stats
                {
                    let mut stats = worker_stats.write().await;
                    if let Some(worker_stat) = stats.get_mut(&worker_id) {
                        worker_stat.current_file = Some(item.file_path.clone());
                        worker_stat.is_active = true;
                        worker_stat.last_activity = Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        );
                    }
                }

                // Process the file
                progress.start_file();

                let result = Self::process_file_item(
                    worker_id,
                    item,
                    &pipelines,
                    &language_detector,
                    &current_memory,
                    &server_manager,
                    &call_graph_cache,
                    &definition_cache,
                )
                .await;

                // Update stats based on result
                {
                    let mut stats = worker_stats.write().await;
                    if let Some(worker_stat) = stats.get_mut(&worker_id) {
                        worker_stat.current_file = None;
                        worker_stat.is_active = false;

                        match result {
                            Ok((bytes, symbols)) => {
                                worker_stat.files_processed += 1;
                                worker_stat.bytes_processed += bytes;
                                worker_stat.symbols_extracted += symbols;
                                progress.complete_file(bytes, symbols);
                            }
                            Err(e) => {
                                worker_stat.errors_encountered += 1;
                                progress.fail_file(&format!("Worker {worker_id}: {e}"));
                            }
                        }
                    }
                }

                // Small yield to allow other tasks to run
                tokio::task::yield_now().await;
            }

            progress.remove_worker();
            debug!("Worker {} shut down", worker_id);
        });

        Ok(handle)
    }

    /// Process a single file item with the appropriate pipeline and LSP server
    #[allow(clippy::too_many_arguments)]
    async fn process_file_item(
        worker_id: usize,
        item: QueueItem,
        pipelines: &Arc<RwLock<HashMap<Language, IndexingPipeline>>>,
        language_detector: &Arc<LanguageDetector>,
        current_memory: &Arc<AtomicU64>,
        server_manager: &Arc<SingleServerManager>,
        call_graph_cache: &Arc<CallGraphCache>,
        definition_cache: &Arc<LspCache<DefinitionInfo>>,
    ) -> Result<(u64, u64)> {
        let file_path = &item.file_path;

        // Detect language if not provided
        let language = if let Some(hint) = &item.language_hint {
            Language::from_str(hint).unwrap_or_else(|| {
                language_detector
                    .detect(file_path)
                    .unwrap_or(Language::Unknown)
            })
        } else {
            language_detector
                .detect(file_path)
                .unwrap_or(Language::Unknown)
        };

        debug!(
            "Worker {} processing {:?} as {:?}",
            worker_id, file_path, language
        );

        // Estimate memory usage for this file
        let file_size = item.estimated_size.unwrap_or(1024);
        let estimated_memory = file_size * 2; // Rough estimate: 2x file size for processing

        current_memory.fetch_add(estimated_memory, Ordering::Relaxed);

        // First, use the existing pipeline to extract symbols from the file
        let symbols_result = {
            let mut pipelines_write = pipelines.write().await;
            let pipeline = pipelines_write.entry(language).or_insert_with(|| {
                IndexingPipeline::new(language).unwrap_or_else(|_| {
                    // Fallback to minimal pipeline if creation fails
                    IndexingPipeline::new(Language::Unknown)
                        .expect("Failed to create fallback pipeline")
                })
            });

            pipeline.process_file(file_path).await
        };

        // Process LSP indexing if pipeline succeeded
        let result = match symbols_result {
            Ok(pipeline_result) => {
                // Now, for each symbol found, query the LSP server for call hierarchy
                // This is the core of what makes indexing actually useful
                let mut total_lsp_calls = 0u64;

                // Only process LSP if we have a supported language and server
                if language != Language::Unknown {
                    // Collect all symbols from the different categories
                    let mut all_symbols = Vec::new();
                    for symbols in pipeline_result.symbols.values() {
                        all_symbols.extend(symbols.iter().cloned());
                    }

                    // Process symbols with LSP to pre-warm the cache
                    total_lsp_calls = Self::index_symbols_with_lsp(
                        worker_id,
                        file_path,
                        &all_symbols,
                        language,
                        server_manager,
                        call_graph_cache,
                        definition_cache,
                    )
                    .await
                    .unwrap_or(0);
                }

                Ok((
                    pipeline_result.bytes_processed,
                    pipeline_result.symbols_found + total_lsp_calls,
                ))
            }
            Err(e) => Err(anyhow!("Failed to process {:?}: {}", file_path, e)),
        };

        // Release memory estimate
        current_memory.fetch_sub(estimated_memory, Ordering::Relaxed);

        result
    }

    /// Index symbols by calling LSP servers to pre-warm the cache
    async fn index_symbols_with_lsp(
        worker_id: usize,
        file_path: &Path,
        symbols: &[SymbolInfo],
        language: Language,
        server_manager: &Arc<SingleServerManager>,
        _call_graph_cache: &Arc<CallGraphCache>,
        _definition_cache: &Arc<LspCache<DefinitionInfo>>,
    ) -> Result<u64> {
        use std::time::Duration;
        use tokio::time::timeout;

        let mut indexed_count = 0u64;

        // Get the LSP server for this language
        let server_instance =
            match timeout(Duration::from_secs(30), server_manager.get_server(language)).await {
                Ok(Ok(server)) => server,
                Ok(Err(e)) => {
                    debug!(
                        "Worker {}: Failed to get LSP server for {:?}: {}",
                        worker_id, language, e
                    );
                    return Ok(0);
                }
                Err(_) => {
                    debug!(
                        "Worker {}: Timeout getting LSP server for {:?}",
                        worker_id, language
                    );
                    return Ok(0);
                }
            };

        // Lock the server instance to access the LspServer
        let server_guard = match timeout(Duration::from_secs(5), server_instance.lock()).await {
            Ok(guard) => guard,
            Err(_) => {
                debug!(
                    "Worker {}: Timeout acquiring server lock for {:?}",
                    worker_id, language
                );
                return Ok(0);
            }
        };

        // Wait for server to be ready (especially important for rust-analyzer)
        let max_retries = 5;
        let retry_delay = Duration::from_secs(3);

        for symbol in symbols {
            // Skip non-function symbols for now (can expand later)
            let kind_lower = symbol.kind.to_lowercase();
            if !kind_lower.contains("function") && !kind_lower.contains("method") {
                continue;
            }

            // Use 1-based indexing (LSP uses 0-based, but our call_hierarchy method handles the conversion)
            let line = symbol.line;
            let column = symbol.column;

            // Try to get call hierarchy with retries
            let mut retry_count = 0;
            let mut call_hierarchy_result = None;

            while retry_count < max_retries {
                match timeout(
                    Duration::from_secs(10),
                    server_guard.server.call_hierarchy(file_path, line, column),
                )
                .await
                {
                    Ok(Ok(result)) => {
                        // Check if we got meaningful data
                        if let Some(obj) = result.as_object() {
                            // Check if the result contains actual call hierarchy data
                            if obj.contains_key("incoming") || obj.contains_key("outgoing") {
                                call_hierarchy_result = Some(result);
                                break;
                            }
                        }
                        // Empty or incomplete response - server might still be indexing
                        debug!(
                            "Worker {}: Empty/incomplete call hierarchy for {} at {}:{}, retry {}/{}",
                            worker_id, symbol.name, line, column, retry_count + 1, max_retries
                        );
                    }
                    Ok(Err(e)) => {
                        debug!(
                            "Worker {}: LSP error for {} at {}:{}: {}",
                            worker_id, symbol.name, line, column, e
                        );
                    }
                    Err(_) => {
                        debug!(
                            "Worker {}: Timeout getting call hierarchy for {} at {}:{}",
                            worker_id, symbol.name, line, column
                        );
                    }
                }

                retry_count += 1;
                if retry_count < max_retries {
                    tokio::time::sleep(retry_delay).await;
                }
            }

            // If we got call hierarchy data, it's now cached
            if let Some(_result) = call_hierarchy_result {
                indexed_count += 1;

                // The call hierarchy is already cached by the LspServer's call_hierarchy method
                // We can optionally store it in our call_graph_cache as well
                // For now, just count it as indexed since the LspServer handles caching

                debug!(
                    "Worker {}: Successfully indexed call hierarchy for {} at {}:{}",
                    worker_id, symbol.name, line, column
                );
            }
        }

        debug!(
            "Worker {}: Indexed {} LSP call hierarchies for {:?}",
            worker_id, indexed_count, file_path
        );

        Ok(indexed_count)
    }

    /// Shutdown all workers gracefully
    async fn shutdown_workers(&self) -> Result<()> {
        let mut handles = self.worker_handles.write().await;

        if handles.is_empty() {
            return Ok(());
        }

        debug!("Shutting down {} workers...", handles.len());

        // Wait for workers to finish with timeout
        let shutdown_timeout = Duration::from_secs(10);
        let mut shutdown_futures = Vec::new();

        for handle in handles.drain(..) {
            shutdown_futures.push(handle);
        }

        // Wait for all workers with timeout
        match timeout(
            shutdown_timeout,
            futures::future::join_all(shutdown_futures),
        )
        .await
        {
            Ok(_) => {
                debug!("All workers shut down gracefully");
            }
            Err(_) => {
                warn!("Worker shutdown timed out after {:?}", shutdown_timeout);
            }
        }

        Ok(())
    }
}

impl Drop for IndexingManager {
    fn drop(&mut self) {
        // Signal shutdown
        self.shutdown_signal.store(true, Ordering::Relaxed);
        debug!("IndexingManager dropped - shutdown signal sent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_types::LspOperation;
    use crate::call_graph_cache::CallGraphCacheConfig;
    use crate::lsp_cache::LspCacheConfig;
    use crate::lsp_registry::LspRegistry;
    use std::fs;
    use std::time::Duration;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_manager_lifecycle() {
        let config = ManagerConfig {
            max_workers: 2,
            memory_budget_bytes: 1024 * 1024, // 1MB
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        // Create mock LSP dependencies for testing
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        // Test initial state
        assert!(matches!(manager.get_status().await, ManagerStatus::Idle));

        // Create test directory with some files
        let temp_dir = tempdir().unwrap();
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() {}\n").unwrap();

        // Start indexing
        manager
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Give it time to start
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status = manager.get_status().await;
        assert!(matches!(
            status,
            ManagerStatus::Indexing | ManagerStatus::Discovering
        ));

        // Stop indexing
        manager.stop_indexing().await.unwrap();
        assert!(matches!(
            manager.get_status().await,
            ManagerStatus::Shutdown
        ));
    }

    #[test]
    fn test_pattern_matching() {
        // Test exclusion patterns
        assert!(IndexingManager::matches_pattern(
            "/path/node_modules/file.js",
            "*node_modules*"
        ));
        assert!(IndexingManager::matches_pattern("test.tmp", "*.tmp"));
        assert!(!IndexingManager::matches_pattern("test.rs", "*.tmp"));

        // Test exact matches
        assert!(IndexingManager::matches_pattern("exact_match", "exact"));
        assert!(!IndexingManager::matches_pattern("no_match", "different"));
    }

    #[test]
    fn test_priority_determination() {
        use std::path::Path;

        // Test high priority languages
        let rust_priority =
            IndexingManager::determine_priority(Path::new("main.rs"), Language::Rust);
        assert_eq!(rust_priority, Priority::High);

        // Test medium priority
        let js_priority =
            IndexingManager::determine_priority(Path::new("script.js"), Language::JavaScript);
        assert_eq!(js_priority, Priority::Medium);

        // Test low priority
        let unknown_priority =
            IndexingManager::determine_priority(Path::new("data.txt"), Language::Unknown);
        assert_eq!(unknown_priority, Priority::Low);
    }

    #[tokio::test]
    async fn test_memory_pressure_detection() {
        let config = ManagerConfig {
            memory_budget_bytes: 1000,
            memory_pressure_threshold: 0.8,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        // Initially no pressure
        assert!(!manager.is_memory_pressure());

        // Simulate memory usage above threshold
        manager.current_memory_usage.store(850, Ordering::Relaxed); // 85% of 1000
        assert!(manager.is_memory_pressure());

        // Back below threshold
        manager.current_memory_usage.store(700, Ordering::Relaxed); // 70% of 1000
        assert!(!manager.is_memory_pressure());
    }

    #[tokio::test]
    async fn test_file_exclusion_patterns() {
        let temp_dir = tempdir().unwrap();
        let root = temp_dir.path();

        // Create various files
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();

        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("target/debug/app"), "binary").unwrap();
        fs::write(root.join("node_modules/package.json"), "{}").unwrap();
        fs::write(root.join("temp.tmp"), "temp").unwrap();
        fs::write(root.join("debug.log"), "log").unwrap();

        let patterns = vec![
            "*/target/*".to_string(),
            "*/node_modules/*".to_string(),
            "*.tmp".to_string(),
            "*.log".to_string(),
        ];

        // Test exclusions
        assert!(IndexingManager::should_exclude_file(
            &root.join("target/debug/app"),
            &patterns
        ));
        assert!(IndexingManager::should_exclude_file(
            &root.join("node_modules/package.json"),
            &patterns
        ));
        assert!(IndexingManager::should_exclude_file(
            &root.join("temp.tmp"),
            &patterns
        ));
        assert!(IndexingManager::should_exclude_file(
            &root.join("debug.log"),
            &patterns
        ));

        // Test inclusions
        assert!(!IndexingManager::should_exclude_file(
            &root.join("src/main.rs"),
            &patterns
        ));
    }

    #[tokio::test]
    async fn test_file_inclusion_patterns() {
        let patterns = vec![
            "*.rs".to_string(),
            "*.ts".to_string(),
            "*/src/*".to_string(),
        ];

        assert!(IndexingManager::should_include_file(
            Path::new("main.rs"),
            &patterns
        ));
        assert!(IndexingManager::should_include_file(
            Path::new("script.ts"),
            &patterns
        ));
        assert!(IndexingManager::should_include_file(
            Path::new("project/src/lib.rs"),
            &patterns
        ));
        assert!(!IndexingManager::should_include_file(
            Path::new("data.txt"),
            &patterns
        ));
    }

    #[tokio::test]
    async fn test_worker_statistics_tracking() {
        let config = ManagerConfig {
            max_workers: 2,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        // Initially no workers
        let stats = manager.get_worker_stats().await;
        assert!(stats.is_empty());

        // Create temp directory with test file
        let temp_dir = tempdir().unwrap();
        fs::write(temp_dir.path().join("test.rs"), "fn main() {}").unwrap();

        // Start indexing to create workers
        manager
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Give workers time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        let stats = manager.get_worker_stats().await;
        assert_eq!(stats.len(), 2); // Should have 2 workers

        for stat in &stats {
            assert!(stat.worker_id >= 1);
            // These are u64, no need to check >= 0
            // Just verify they exist (implicit by the struct)
        }

        manager.stop_indexing().await.unwrap();
    }

    #[tokio::test]
    async fn test_pause_resume_functionality() {
        let config = ManagerConfig {
            max_workers: 1,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        let temp_dir = tempdir().unwrap();
        fs::write(temp_dir.path().join("test.rs"), "fn main() {}").unwrap();

        // Start indexing
        manager
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Test pause
        let pause_result = manager.pause_indexing().await;
        assert!(pause_result.is_ok());

        let status = manager.get_status().await;
        assert!(matches!(status, ManagerStatus::Paused));

        // Test resume
        let resume_result = manager.resume_indexing().await;
        assert!(resume_result.is_ok());

        let status = manager.get_status().await;
        assert!(matches!(status, ManagerStatus::Indexing));

        manager.stop_indexing().await.unwrap();
    }

    #[tokio::test]
    async fn test_queue_integration() {
        let config = ManagerConfig {
            max_queue_size: 10,
            max_workers: 1,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        // Initially empty queue
        let snapshot = manager.get_queue_snapshot().await;
        assert_eq!(snapshot.total_items, 0);

        let temp_dir = tempdir().unwrap();
        for i in 0..5 {
            fs::write(
                temp_dir.path().join(format!("lib_{}.rs", i)),
                "fn main() {}",
            )
            .unwrap();
        }

        // Start indexing
        manager
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Wait for files to be discovered and processed
        let mut found_items = false;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let snapshot = manager.get_queue_snapshot().await;
            if snapshot.total_items > 0 {
                found_items = true;
                break;
            }
            let progress = manager.get_progress().await;
            if progress.total_files >= 5 {
                break;
            }
        }

        // Either we found items in the queue, or all files were processed quickly
        // Check that files were at least discovered
        let final_progress = manager.get_progress().await;
        assert!(found_items || final_progress.total_files >= 5);

        manager.stop_indexing().await.unwrap();
    }

    #[tokio::test]
    async fn test_progress_tracking() {
        let config = ManagerConfig {
            max_workers: 2,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        let temp_dir = tempdir().unwrap();
        for i in 0..3 {
            fs::write(
                temp_dir.path().join(format!("file_{}.rs", i)),
                format!("fn func_{}() {{}}", i),
            )
            .unwrap();
        }

        // Start indexing
        manager
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Monitor progress
        let mut progress_updates = 0;
        let start_time = Instant::now();

        while start_time.elapsed() < Duration::from_secs(5) {
            let progress = manager.get_progress().await;

            if progress.total_files > 0 {
                progress_updates += 1;

                // Basic progress invariants
                assert!(
                    progress.processed_files + progress.failed_files + progress.skipped_files
                        <= progress.total_files
                );
                // active_workers is usize, no need to check >= 0

                if progress.is_complete() {
                    break;
                }
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert!(progress_updates > 0);

        let final_progress = manager.get_progress().await;
        assert!(final_progress.total_files >= 3); // Should have found our test files

        manager.stop_indexing().await.unwrap();
    }

    #[tokio::test]
    async fn test_incremental_mode_detection() {
        let temp_dir = tempdir().unwrap();
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() {}").unwrap();

        // First run - full indexing
        let config = ManagerConfig {
            incremental_mode: true,
            max_workers: 1,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager1 = IndexingManager::new(
            config.clone(),
            language_detector.clone(),
            server_manager.clone(),
            call_graph_cache.clone(),
            definition_cache.clone(),
        );

        manager1
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Wait for completion
        tokio::time::sleep(Duration::from_millis(500)).await;

        let progress1 = manager1.get_progress().await;
        manager1.stop_indexing().await.unwrap();

        // Second run - incremental (should detect no changes if file hasn't changed)
        let manager2 = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );
        manager2
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;

        let progress2 = manager2.get_progress().await;
        manager2.stop_indexing().await.unwrap();

        // In incremental mode, second run might process fewer or equal files
        assert!(progress2.processed_files <= progress1.processed_files);
    }

    #[test]
    fn test_glob_pattern_matching_edge_cases() {
        // Single wildcard
        assert!(IndexingManager::matches_pattern("hello.txt", "*.txt"));
        assert!(IndexingManager::matches_pattern("test", "*test"));
        assert!(IndexingManager::matches_pattern("prefix_test", "*test"));
        assert!(!IndexingManager::matches_pattern("hello.rs", "*.txt"));

        // Multiple wildcards
        assert!(IndexingManager::matches_pattern(
            "path/to/file.txt",
            "*/*/file.txt"
        ));
        assert!(IndexingManager::matches_pattern("a_b_c", "*_*_*"));
        assert!(!IndexingManager::matches_pattern("a_b", "*_*_*"));

        // No wildcards (substring matching)
        assert!(IndexingManager::matches_pattern("hello world", "hello"));
        assert!(IndexingManager::matches_pattern("testing", "test"));
        assert!(!IndexingManager::matches_pattern("hello", "world"));

        // Edge cases
        assert!(IndexingManager::matches_pattern("", ""));
        assert!(IndexingManager::matches_pattern("anything", "*"));
        assert!(!IndexingManager::matches_pattern("", "something"));
    }

    #[tokio::test]
    async fn test_error_handling_during_indexing() {
        let temp_dir = tempdir().unwrap();

        // Create a valid file
        fs::write(temp_dir.path().join("valid.rs"), "fn main() {}").unwrap();

        // Create a file that will cause issues (binary content)
        fs::write(
            temp_dir.path().join("binary.rs"),
            b"\x00\x01\x02\x03\xff\xfe",
        )
        .unwrap();

        let config = ManagerConfig {
            max_workers: 1,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        manager
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Wait for processing
        tokio::time::sleep(Duration::from_millis(1000)).await;

        let progress = manager.get_progress().await;
        manager.stop_indexing().await.unwrap();

        // Should have processed at least one file and possibly failed on others
        assert!(progress.processed_files > 0 || progress.failed_files > 0);
        assert!(progress.total_files >= 2);
    }

    #[tokio::test]
    async fn test_language_filtering() {
        let temp_dir = tempdir().unwrap();

        // Create files in different languages
        fs::write(temp_dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(temp_dir.path().join("script.js"), "console.log('hello');").unwrap();
        fs::write(temp_dir.path().join("app.py"), "print('hello')").unwrap();

        let config = ManagerConfig {
            enabled_languages: vec!["rust".to_string()], // Only process Rust files
            max_workers: 1,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        manager
            .start_indexing(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;

        let progress = manager.get_progress().await;
        manager.stop_indexing().await.unwrap();

        // Should have processed only Rust files, so fewer than total files created
        assert!(progress.processed_files > 0);
        // The exact count depends on language detection and filtering implementation
    }

    #[tokio::test]
    async fn test_manager_from_indexing_config() {
        let mut indexing_config = IndexingConfig::default();
        indexing_config.enabled = true;
        indexing_config.max_workers = 3;
        indexing_config.memory_budget_mb = 128;
        indexing_config.max_queue_size = 500;

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = IndexingManager::from_indexing_config(
            &indexing_config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        );

        // Verify configuration was properly converted
        assert_eq!(manager.config.max_workers, 3);
        assert_eq!(manager.config.memory_budget_bytes, 128 * 1024 * 1024);
        assert_eq!(manager.config.max_queue_size, 500);
    }

    #[tokio::test]
    async fn test_concurrent_start_stop_operations() {
        let config = ManagerConfig {
            max_workers: 2,
            ..ManagerConfig::default()
        };

        let language_detector = Arc::new(LanguageDetector::new());
        let registry = Arc::new(LspRegistry::new().expect("Failed to create LspRegistry"));
        let server_manager = Arc::new(SingleServerManager::new(registry));
        let cache_config = CallGraphCacheConfig::default();
        let call_graph_cache = Arc::new(CallGraphCache::new(cache_config));
        let lsp_cache_config = LspCacheConfig::default();
        let definition_cache = Arc::new(
            LspCache::<DefinitionInfo>::new(LspOperation::Definition, lsp_cache_config)
                .expect("Failed to create LspCache"),
        );
        let manager = Arc::new(IndexingManager::new(
            config,
            language_detector,
            server_manager,
            call_graph_cache,
            definition_cache,
        ));

        let temp_dir = tempdir().unwrap();
        fs::write(temp_dir.path().join("test.rs"), "fn main() {}").unwrap();

        // Test starting multiple times (should fail after first)
        let manager1 = Arc::clone(&manager);
        let path1 = temp_dir.path().to_path_buf();
        let start_result1 = manager1.start_indexing(path1).await;
        assert!(start_result1.is_ok());

        let manager2 = Arc::clone(&manager);
        let path2 = temp_dir.path().to_path_buf();
        let start_result2 = manager2.start_indexing(path2).await;
        assert!(start_result2.is_err()); // Should fail - already running

        // Stop and verify
        manager.stop_indexing().await.unwrap();

        let status = manager.get_status().await;
        assert!(matches!(status, ManagerStatus::Shutdown));
    }
}
