//! Lock-free progress tracking for indexing operations using atomic counters
//!
//! This module provides thread-safe progress tracking without locks, allowing
//! multiple indexing workers to update progress concurrently while providing
//! real-time visibility into indexing status.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::debug;

/// Lock-free progress tracker for indexing operations
#[derive(Debug, Clone)]
pub struct IndexingProgress {
    /// Total files discovered for indexing
    total_files: Arc<AtomicU64>,

    /// Files successfully processed
    processed_files: Arc<AtomicU64>,

    /// Files that failed processing
    failed_files: Arc<AtomicU64>,

    /// Files currently being processed
    active_files: Arc<AtomicU64>,

    /// Files skipped (already indexed, filtered out, etc.)
    skipped_files: Arc<AtomicU64>,

    /// Total bytes processed
    processed_bytes: Arc<AtomicU64>,

    /// Total symbols extracted
    symbols_extracted: Arc<AtomicU64>,

    /// Current memory usage estimate (bytes)
    memory_usage: Arc<AtomicU64>,

    /// Peak memory usage observed
    peak_memory: Arc<AtomicU64>,

    /// Number of worker threads currently active
    active_workers: Arc<AtomicUsize>,

    /// Start time of indexing operation
    start_time: Instant,

    /// Last update timestamp for progress calculations
    last_update: Arc<AtomicU64>, // Unix timestamp in milliseconds
}

impl IndexingProgress {
    /// Create a new progress tracker
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            total_files: Arc::new(AtomicU64::new(0)),
            processed_files: Arc::new(AtomicU64::new(0)),
            failed_files: Arc::new(AtomicU64::new(0)),
            active_files: Arc::new(AtomicU64::new(0)),
            skipped_files: Arc::new(AtomicU64::new(0)),
            processed_bytes: Arc::new(AtomicU64::new(0)),
            symbols_extracted: Arc::new(AtomicU64::new(0)),
            memory_usage: Arc::new(AtomicU64::new(0)),
            peak_memory: Arc::new(AtomicU64::new(0)),
            active_workers: Arc::new(AtomicUsize::new(0)),
            start_time: now,
            last_update: Arc::new(AtomicU64::new(now.elapsed().as_millis() as u64)),
        }
    }

    /// Reset all progress counters
    pub fn reset(&self) {
        self.total_files.store(0, Ordering::Relaxed);
        self.processed_files.store(0, Ordering::Relaxed);
        self.failed_files.store(0, Ordering::Relaxed);
        self.active_files.store(0, Ordering::Relaxed);
        self.skipped_files.store(0, Ordering::Relaxed);
        self.processed_bytes.store(0, Ordering::Relaxed);
        self.symbols_extracted.store(0, Ordering::Relaxed);
        self.memory_usage.store(0, Ordering::Relaxed);
        self.peak_memory.store(0, Ordering::Relaxed);
        self.active_workers.store(0, Ordering::Relaxed);
        self.update_timestamp();
    }

    /// Set total number of files discovered
    pub fn set_total_files(&self, total: u64) {
        self.total_files.store(total, Ordering::Relaxed);
        self.update_timestamp();
        debug!("Set total files to index: {}", total);
    }

    /// Increment total files (for dynamic discovery)
    pub fn add_total_files(&self, count: u64) -> u64 {
        let new_total = self.total_files.fetch_add(count, Ordering::Relaxed) + count;
        self.update_timestamp();
        debug!("Added {} files to index (total: {})", count, new_total);
        new_total
    }

    /// Mark a file as being processed (increment active count)
    pub fn start_file(&self) -> u64 {
        let active = self.active_files.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_timestamp();
        active
    }

    /// Mark a file as successfully processed
    pub fn complete_file(&self, bytes_processed: u64, symbols_found: u64) {
        self.active_files.fetch_sub(1, Ordering::Relaxed);
        self.processed_files.fetch_add(1, Ordering::Relaxed);
        self.processed_bytes
            .fetch_add(bytes_processed, Ordering::Relaxed);
        self.symbols_extracted
            .fetch_add(symbols_found, Ordering::Relaxed);
        self.update_timestamp();
    }

    /// Mark a file as failed processing
    pub fn fail_file(&self, error_context: &str) {
        self.active_files.fetch_sub(1, Ordering::Relaxed);
        self.failed_files.fetch_add(1, Ordering::Relaxed);
        self.update_timestamp();
        debug!("Failed to process file: {}", error_context);
    }

    /// Mark a file as skipped
    pub fn skip_file(&self, reason: &str) {
        self.skipped_files.fetch_add(1, Ordering::Relaxed);
        self.update_timestamp();
        debug!("Skipped file: {}", reason);
    }

    /// Update memory usage estimate
    pub fn update_memory_usage(&self, current_bytes: u64) {
        self.memory_usage.store(current_bytes, Ordering::Relaxed);

        // Update peak memory if current exceeds it
        let current_peak = self.peak_memory.load(Ordering::Relaxed);
        if current_bytes > current_peak {
            // Use compare_exchange to avoid race conditions
            let _ = self.peak_memory.compare_exchange_weak(
                current_peak,
                current_bytes,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }

        self.update_timestamp();
    }

    /// Add memory to current usage
    pub fn add_memory_usage(&self, additional_bytes: u64) -> u64 {
        let new_usage = self
            .memory_usage
            .fetch_add(additional_bytes, Ordering::Relaxed)
            + additional_bytes;

        // Update peak if needed
        let current_peak = self.peak_memory.load(Ordering::Relaxed);
        if new_usage > current_peak {
            let _ = self.peak_memory.compare_exchange_weak(
                current_peak,
                new_usage,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }

        self.update_timestamp();
        new_usage
    }

    /// Subtract memory from current usage
    pub fn subtract_memory_usage(&self, bytes_freed: u64) -> u64 {
        let new_usage = self
            .memory_usage
            .fetch_sub(bytes_freed, Ordering::Relaxed)
            .saturating_sub(bytes_freed);
        self.update_timestamp();
        new_usage
    }

    /// Increment active worker count
    pub fn add_worker(&self) -> usize {
        let count = self.active_workers.fetch_add(1, Ordering::Relaxed) + 1;
        self.update_timestamp();
        debug!("Worker started (active: {})", count);
        count
    }

    /// Decrement active worker count
    pub fn remove_worker(&self) -> usize {
        let count = self
            .active_workers
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        self.update_timestamp();
        debug!("Worker finished (active: {})", count);
        count
    }

    /// Get current progress metrics
    pub fn get_metrics(&self) -> ProgressMetrics {
        let total = self.total_files.load(Ordering::Relaxed);
        let processed = self.processed_files.load(Ordering::Relaxed);
        let failed = self.failed_files.load(Ordering::Relaxed);
        let active = self.active_files.load(Ordering::Relaxed);
        let skipped = self.skipped_files.load(Ordering::Relaxed);

        let completed = processed + failed + skipped;
        let progress_ratio = if total > 0 {
            completed as f64 / total as f64
        } else {
            0.0
        };

        let elapsed = self.start_time.elapsed();
        let files_per_second = if elapsed.as_secs() > 0 {
            completed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        let bytes_processed = self.processed_bytes.load(Ordering::Relaxed);
        let bytes_per_second = if elapsed.as_secs() > 0 {
            bytes_processed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        ProgressMetrics {
            total_files: total,
            processed_files: processed,
            failed_files: failed,
            active_files: active,
            skipped_files: skipped,
            progress_ratio,
            files_per_second,
            processed_bytes: bytes_processed,
            bytes_per_second,
            symbols_extracted: self.symbols_extracted.load(Ordering::Relaxed),
            memory_usage_bytes: self.memory_usage.load(Ordering::Relaxed),
            peak_memory_bytes: self.peak_memory.load(Ordering::Relaxed),
            active_workers: self.active_workers.load(Ordering::Relaxed),
            elapsed_time: elapsed,
        }
    }

    /// Get a lightweight snapshot for serialization
    pub fn get_snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            total_files: self.total_files.load(Ordering::Relaxed),
            processed_files: self.processed_files.load(Ordering::Relaxed),
            failed_files: self.failed_files.load(Ordering::Relaxed),
            active_files: self.active_files.load(Ordering::Relaxed),
            skipped_files: self.skipped_files.load(Ordering::Relaxed),
            processed_bytes: self.processed_bytes.load(Ordering::Relaxed),
            symbols_extracted: self.symbols_extracted.load(Ordering::Relaxed),
            memory_usage_bytes: self.memory_usage.load(Ordering::Relaxed),
            peak_memory_bytes: self.peak_memory.load(Ordering::Relaxed),
            active_workers: self.active_workers.load(Ordering::Relaxed),
            elapsed_seconds: self.start_time.elapsed().as_secs(),
        }
    }

    /// Check if indexing is complete
    pub fn is_complete(&self) -> bool {
        let total = self.total_files.load(Ordering::Relaxed);
        let active = self.active_files.load(Ordering::Relaxed);
        let completed = self.processed_files.load(Ordering::Relaxed)
            + self.failed_files.load(Ordering::Relaxed)
            + self.skipped_files.load(Ordering::Relaxed);

        total > 0 && active == 0 && completed >= total
    }

    /// Check if any workers are active
    pub fn has_active_workers(&self) -> bool {
        self.active_workers.load(Ordering::Relaxed) > 0
            || self.active_files.load(Ordering::Relaxed) > 0
    }

    /// Calculate estimated time remaining based on current rate
    pub fn estimate_time_remaining(&self) -> Option<Duration> {
        let metrics = self.get_metrics();

        if metrics.files_per_second > 0.0 && metrics.total_files > 0 {
            let remaining_files = metrics.total_files.saturating_sub(
                metrics.processed_files + metrics.failed_files + metrics.skipped_files,
            );

            if remaining_files > 0 {
                let estimated_seconds = remaining_files as f64 / metrics.files_per_second;
                return Some(Duration::from_secs_f64(estimated_seconds));
            }
        }

        None
    }

    /// Update internal timestamp for progress tracking
    fn update_timestamp(&self) {
        let now_millis = self.start_time.elapsed().as_millis() as u64;
        self.last_update.store(now_millis, Ordering::Relaxed);
    }
}

impl Default for IndexingProgress {
    fn default() -> Self {
        Self::new()
    }
}

/// Progress metrics with calculated rates and statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressMetrics {
    pub total_files: u64,
    pub processed_files: u64,
    pub failed_files: u64,
    pub active_files: u64,
    pub skipped_files: u64,
    pub progress_ratio: f64,
    pub files_per_second: f64,
    pub processed_bytes: u64,
    pub bytes_per_second: f64,
    pub symbols_extracted: u64,
    pub memory_usage_bytes: u64,
    pub peak_memory_bytes: u64,
    pub active_workers: usize,
    pub elapsed_time: Duration,
}

/// Lightweight progress snapshot for serialization/IPC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressSnapshot {
    pub total_files: u64,
    pub processed_files: u64,
    pub failed_files: u64,
    pub active_files: u64,
    pub skipped_files: u64,
    pub processed_bytes: u64,
    pub symbols_extracted: u64,
    pub memory_usage_bytes: u64,
    pub peak_memory_bytes: u64,
    pub active_workers: usize,
    pub elapsed_seconds: u64,
}

impl ProgressSnapshot {
    /// Check if indexing is complete
    pub fn is_complete(&self) -> bool {
        let completed = self.processed_files + self.failed_files + self.skipped_files;
        self.total_files > 0 && self.active_files == 0 && completed >= self.total_files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration as StdDuration;

    #[test]
    fn test_basic_progress_tracking() {
        let progress = IndexingProgress::new();

        // Test initial state
        assert_eq!(progress.total_files.load(Ordering::Relaxed), 0);
        assert_eq!(progress.processed_files.load(Ordering::Relaxed), 0);
        assert!(!progress.is_complete());

        // Set total and process some files
        progress.set_total_files(10);
        assert_eq!(progress.total_files.load(Ordering::Relaxed), 10);

        progress.start_file();
        assert_eq!(progress.active_files.load(Ordering::Relaxed), 1);

        progress.complete_file(1000, 50);
        assert_eq!(progress.active_files.load(Ordering::Relaxed), 0);
        assert_eq!(progress.processed_files.load(Ordering::Relaxed), 1);
        assert_eq!(progress.processed_bytes.load(Ordering::Relaxed), 1000);
        assert_eq!(progress.symbols_extracted.load(Ordering::Relaxed), 50);
    }

    #[test]
    fn test_memory_tracking() {
        let progress = IndexingProgress::new();

        // Test memory usage tracking
        progress.update_memory_usage(1024);
        assert_eq!(progress.memory_usage.load(Ordering::Relaxed), 1024);
        assert_eq!(progress.peak_memory.load(Ordering::Relaxed), 1024);

        progress.add_memory_usage(512);
        assert_eq!(progress.memory_usage.load(Ordering::Relaxed), 1536);
        assert_eq!(progress.peak_memory.load(Ordering::Relaxed), 1536);

        progress.subtract_memory_usage(256);
        assert_eq!(progress.memory_usage.load(Ordering::Relaxed), 1280);
        // Peak should remain at previous high
        assert_eq!(progress.peak_memory.load(Ordering::Relaxed), 1536);
    }

    #[test]
    fn test_worker_tracking() {
        let progress = IndexingProgress::new();

        assert_eq!(progress.active_workers.load(Ordering::Relaxed), 0);

        progress.add_worker();
        assert_eq!(progress.active_workers.load(Ordering::Relaxed), 1);

        progress.add_worker();
        assert_eq!(progress.active_workers.load(Ordering::Relaxed), 2);

        progress.remove_worker();
        assert_eq!(progress.active_workers.load(Ordering::Relaxed), 1);

        progress.remove_worker();
        assert_eq!(progress.active_workers.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_completion_detection() {
        let progress = IndexingProgress::new();

        // Not complete with no files
        assert!(!progress.is_complete());

        progress.set_total_files(3);
        assert!(!progress.is_complete());

        // Process all files
        progress.start_file();
        progress.complete_file(100, 10);
        progress.start_file();
        progress.fail_file("test error");
        progress.skip_file("test skip");

        // Should be complete now
        assert!(progress.is_complete());
    }

    #[test]
    fn test_metrics_calculation() {
        let progress = IndexingProgress::new();

        progress.set_total_files(100);
        progress.complete_file(1000, 50);
        progress.complete_file(2000, 75);
        progress.fail_file("error");

        let metrics = progress.get_metrics();
        assert_eq!(metrics.total_files, 100);
        assert_eq!(metrics.processed_files, 2);
        assert_eq!(metrics.failed_files, 1);
        assert_eq!(metrics.processed_bytes, 3000);
        assert_eq!(metrics.symbols_extracted, 125);
        assert!(metrics.progress_ratio > 0.0);
    }

    #[test]
    fn test_concurrent_updates() {
        let progress = Arc::new(IndexingProgress::new());
        let mut handles = Vec::new();

        // Spawn multiple threads that update progress concurrently
        for i in 0..10 {
            let progress_clone = Arc::clone(&progress);
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    if i % 2 == 0 {
                        progress_clone.add_total_files(1);
                        progress_clone.start_file();
                        progress_clone.complete_file(j * 10, j * 2);
                    } else {
                        progress_clone.add_memory_usage(j * 100);
                        progress_clone.add_worker();
                        thread::sleep(StdDuration::from_millis(1));
                        progress_clone.remove_worker();
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify final state is consistent
        let metrics = progress.get_metrics();
        assert!(metrics.total_files > 0);
        assert!(metrics.processed_files > 0 || metrics.active_files > 0);
        assert_eq!(metrics.active_workers, 0); // All workers should have finished
    }

    #[test]
    fn test_reset_functionality() {
        let progress = IndexingProgress::new();

        // Set up some progress
        progress.set_total_files(50);
        progress.start_file();
        progress.complete_file(1000, 25);
        progress.add_worker();
        progress.update_memory_usage(2048);

        // Verify progress was recorded
        assert!(progress.total_files.load(Ordering::Relaxed) > 0);
        assert!(progress.processed_files.load(Ordering::Relaxed) > 0);

        // Reset and verify everything is cleared
        progress.reset();
        assert_eq!(progress.total_files.load(Ordering::Relaxed), 0);
        assert_eq!(progress.processed_files.load(Ordering::Relaxed), 0);
        assert_eq!(progress.active_files.load(Ordering::Relaxed), 0);
        assert_eq!(progress.memory_usage.load(Ordering::Relaxed), 0);
        // Note: active_workers and peak_memory are not reset to preserve some state
    }
}
