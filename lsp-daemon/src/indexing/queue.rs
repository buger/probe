//! Multi-level priority queue for indexing operations
//!
//! This module provides a thread-safe priority queue with three levels:
//! High, Medium, and Low priority. The queue supports O(1) enqueue operations
//! and provides fair scheduling with priority-based dequeuing.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Priority levels for indexing queue items
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    Critical = 3,
    High = 2,
    Medium = 1,
    Low = 0,
}

impl Priority {
    /// Convert priority to numeric value for ordering
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parse priority from string (case-insensitive)
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "critical" | "crit" | "c" | "3" => Some(Priority::Critical),
            "high" | "h" | "2" => Some(Priority::High),
            "medium" | "med" | "m" | "1" => Some(Priority::Medium),
            "low" | "l" | "0" => Some(Priority::Low),
            _ => None,
        }
    }

    /// Get human-readable name
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Critical => "critical",
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        }
    }
}

/// Item in the indexing queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    /// Unique identifier for this item
    pub id: u64,

    /// File path to be processed
    pub file_path: PathBuf,

    /// Priority level
    pub priority: Priority,

    /// Timestamp when item was enqueued (Unix timestamp in milliseconds)
    pub enqueued_at: u64,

    /// Language hint for processing (if known)
    pub language_hint: Option<String>,

    /// Estimated file size in bytes (for memory budget planning)
    pub estimated_size: Option<u64>,

    /// Additional metadata for processing
    pub metadata: serde_json::Value,
}

impl QueueItem {
    /// Create a new queue item with the specified priority
    pub fn new(file_path: PathBuf, priority: Priority) -> Self {
        Self {
            id: generate_item_id(),
            file_path,
            priority,
            enqueued_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            language_hint: None,
            estimated_size: None,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// Create a new critical-priority item
    pub fn critical_priority(file_path: PathBuf) -> Self {
        Self::new(file_path, Priority::Critical)
    }

    /// Create a new high-priority item
    pub fn high_priority(file_path: PathBuf) -> Self {
        Self::new(file_path, Priority::High)
    }

    /// Create a new medium-priority item
    pub fn medium_priority(file_path: PathBuf) -> Self {
        Self::new(file_path, Priority::Medium)
    }

    /// Create a new low-priority item
    pub fn low_priority(file_path: PathBuf) -> Self {
        Self::new(file_path, Priority::Low)
    }

    /// Set language hint
    pub fn with_language_hint(mut self, language: String) -> Self {
        self.language_hint = Some(language);
        self
    }

    /// Set estimated file size
    pub fn with_estimated_size(mut self, size: u64) -> Self {
        self.estimated_size = Some(size);
        self
    }

    /// Set metadata
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Calculate age since enqueue
    pub fn age(&self) -> Duration {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Duration::from_millis(now_millis.saturating_sub(self.enqueued_at))
    }
}

/// Thread-safe multi-level priority queue
#[derive(Debug, Clone)]
pub struct IndexingQueue {
    /// Critical priority queue
    critical_priority: Arc<RwLock<VecDeque<QueueItem>>>,

    /// High priority queue
    high_priority: Arc<RwLock<VecDeque<QueueItem>>>,

    /// Medium priority queue  
    medium_priority: Arc<RwLock<VecDeque<QueueItem>>>,

    /// Low priority queue
    low_priority: Arc<RwLock<VecDeque<QueueItem>>>,

    /// Total items in all queues
    total_items: Arc<AtomicUsize>,

    /// Total items enqueued (for ID generation)
    total_enqueued: Arc<AtomicU64>,

    /// Total items dequeued
    total_dequeued: Arc<AtomicU64>,

    /// Total bytes estimated across all queued items
    estimated_total_bytes: Arc<AtomicU64>,

    /// Maximum queue size (0 = unlimited)
    max_size: usize,

    /// Whether the queue is paused
    paused: Arc<std::sync::atomic::AtomicBool>,

    /// Queue creation time
    created_at: Instant,
}

impl IndexingQueue {
    /// Create a new indexing queue with optional size limit
    pub fn new(max_size: usize) -> Self {
        Self {
            critical_priority: Arc::new(RwLock::new(VecDeque::new())),
            high_priority: Arc::new(RwLock::new(VecDeque::new())),
            medium_priority: Arc::new(RwLock::new(VecDeque::new())),
            low_priority: Arc::new(RwLock::new(VecDeque::new())),
            total_items: Arc::new(AtomicUsize::new(0)),
            total_enqueued: Arc::new(AtomicU64::new(0)),
            total_dequeued: Arc::new(AtomicU64::new(0)),
            estimated_total_bytes: Arc::new(AtomicU64::new(0)),
            max_size,
            paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            created_at: Instant::now(),
        }
    }

    /// Create a new unlimited queue
    pub fn unlimited() -> Self {
        Self::new(0)
    }

    /// Enqueue an item with the specified priority (O(1) operation)
    pub async fn enqueue(&self, item: QueueItem) -> Result<bool> {
        // Check if queue is paused
        if self.paused.load(Ordering::Relaxed) {
            debug!("Queue is paused, rejecting item: {:?}", item.file_path);
            return Ok(false);
        }

        // Check size limit
        if self.max_size > 0 && self.total_items.load(Ordering::Relaxed) >= self.max_size {
            warn!(
                "Queue at maximum capacity ({}), rejecting item: {:?}",
                self.max_size, item.file_path
            );
            return Ok(false);
        }

        let queue = match item.priority {
            Priority::Critical => &self.critical_priority,
            Priority::High => &self.high_priority,
            Priority::Medium => &self.medium_priority,
            Priority::Low => &self.low_priority,
        };

        // Update byte estimate
        if let Some(size) = item.estimated_size {
            self.estimated_total_bytes
                .fetch_add(size, Ordering::Relaxed);
        }

        // Add to appropriate queue
        {
            let mut queue_guard = queue.write().await;
            queue_guard.push_back(item.clone());
        }

        // Update counters
        self.total_items.fetch_add(1, Ordering::Relaxed);
        self.total_enqueued.fetch_add(1, Ordering::Relaxed);

        debug!(
            "Enqueued {} priority item: {:?} (queue size: {})",
            item.priority.as_str(),
            item.file_path,
            self.len()
        );

        Ok(true)
    }

    /// Dequeue the highest priority item available (O(1) average case)
    pub async fn dequeue(&self) -> Option<QueueItem> {
        // Check if queue is paused
        if self.paused.load(Ordering::Relaxed) {
            return None;
        }

        // Try critical priority first, then high, medium, then low
        for (priority, queue) in [
            (Priority::Critical, &self.critical_priority),
            (Priority::High, &self.high_priority),
            (Priority::Medium, &self.medium_priority),
            (Priority::Low, &self.low_priority),
        ] {
            let mut queue_guard = queue.write().await;
            if let Some(item) = queue_guard.pop_front() {
                drop(queue_guard); // Release lock early

                // Update counters
                self.total_items.fetch_sub(1, Ordering::Relaxed);
                self.total_dequeued.fetch_add(1, Ordering::Relaxed);

                // Update byte estimate
                if let Some(size) = item.estimated_size {
                    self.estimated_total_bytes
                        .fetch_sub(size, Ordering::Relaxed);
                }

                debug!(
                    "Dequeued {} priority item: {:?} (queue size: {})",
                    priority.as_str(),
                    item.file_path,
                    self.len()
                );

                return Some(item);
            }
        }

        None
    }

    /// Peek at the next item that would be dequeued without removing it
    pub async fn peek(&self) -> Option<QueueItem> {
        // Try critical priority first, then high, medium, then low
        for queue in [
            &self.critical_priority,
            &self.high_priority,
            &self.medium_priority,
            &self.low_priority,
        ] {
            let queue_guard = queue.read().await;
            if let Some(item) = queue_guard.front() {
                return Some(item.clone());
            }
        }

        None
    }

    /// Get the current length of all queues combined
    pub fn len(&self) -> usize {
        self.total_items.load(Ordering::Relaxed)
    }

    /// Check if all queues are empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get length of a specific priority queue
    pub async fn len_for_priority(&self, priority: Priority) -> usize {
        let queue = match priority {
            Priority::Critical => &self.critical_priority,
            Priority::High => &self.high_priority,
            Priority::Medium => &self.medium_priority,
            Priority::Low => &self.low_priority,
        };

        queue.read().await.len()
    }

    /// Clear all queues
    pub async fn clear(&self) {
        let mut critical = self.critical_priority.write().await;
        let mut high = self.high_priority.write().await;
        let mut medium = self.medium_priority.write().await;
        let mut low = self.low_priority.write().await;

        critical.clear();
        high.clear();
        medium.clear();
        low.clear();

        self.total_items.store(0, Ordering::Relaxed);
        self.estimated_total_bytes.store(0, Ordering::Relaxed);

        debug!("Cleared all queues");
    }

    /// Clear a specific priority queue
    pub async fn clear_priority(&self, priority: Priority) {
        let queue = match priority {
            Priority::Critical => &self.critical_priority,
            Priority::High => &self.high_priority,
            Priority::Medium => &self.medium_priority,
            Priority::Low => &self.low_priority,
        };

        let mut queue_guard = queue.write().await;
        let cleared_count = queue_guard.len();

        // Update byte estimates for cleared items
        for item in queue_guard.iter() {
            if let Some(size) = item.estimated_size {
                self.estimated_total_bytes
                    .fetch_sub(size, Ordering::Relaxed);
            }
        }

        queue_guard.clear();
        self.total_items.fetch_sub(cleared_count, Ordering::Relaxed);

        debug!(
            "Cleared {} items from {} priority queue",
            cleared_count,
            priority.as_str()
        );
    }

    /// Pause the queue (prevents enqueue/dequeue operations)
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        debug!("Queue paused");
    }

    /// Resume the queue
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
        debug!("Queue resumed");
    }

    /// Check if queue is paused
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Get queue metrics
    pub async fn get_metrics(&self) -> QueueMetrics {
        let critical_len = self.len_for_priority(Priority::Critical).await;
        let high_len = self.len_for_priority(Priority::High).await;
        let medium_len = self.len_for_priority(Priority::Medium).await;
        let low_len = self.len_for_priority(Priority::Low).await;

        QueueMetrics {
            total_items: self.len(),
            critical_priority_items: critical_len,
            high_priority_items: high_len,
            medium_priority_items: medium_len,
            low_priority_items: low_len,
            total_enqueued: self.total_enqueued.load(Ordering::Relaxed),
            total_dequeued: self.total_dequeued.load(Ordering::Relaxed),
            estimated_total_bytes: self.estimated_total_bytes.load(Ordering::Relaxed),
            is_paused: self.is_paused(),
            max_size: self.max_size,
            utilization_ratio: if self.max_size > 0 {
                self.len() as f64 / self.max_size as f64
            } else {
                0.0
            },
            age_seconds: self.created_at.elapsed().as_secs(),
        }
    }

    /// Get a lightweight snapshot for serialization
    pub async fn get_snapshot(&self) -> QueueSnapshot {
        let metrics = self.get_metrics().await;

        QueueSnapshot {
            total_items: metrics.total_items,
            critical_priority_items: metrics.critical_priority_items,
            high_priority_items: metrics.high_priority_items,
            medium_priority_items: metrics.medium_priority_items,
            low_priority_items: metrics.low_priority_items,
            estimated_total_bytes: metrics.estimated_total_bytes,
            is_paused: metrics.is_paused,
            utilization_ratio: metrics.utilization_ratio,
        }
    }

    /// Enqueue multiple items in batch for efficiency
    pub async fn enqueue_batch(&self, items: Vec<QueueItem>) -> Result<usize> {
        let mut enqueued_count = 0;

        for item in items {
            if self.enqueue(item).await? {
                enqueued_count += 1;
            }
        }

        debug!("Batch enqueued {} items", enqueued_count);
        Ok(enqueued_count)
    }

    /// Remove items matching a predicate (useful for cleanup)
    pub async fn remove_matching<F>(&self, predicate: F) -> usize
    where
        F: Fn(&QueueItem) -> bool,
    {
        let mut removed_count = 0;

        for queue in [
            &self.critical_priority,
            &self.high_priority,
            &self.medium_priority,
            &self.low_priority,
        ] {
            let mut queue_guard = queue.write().await;
            let original_len = queue_guard.len();

            queue_guard.retain(|item| {
                let should_remove = predicate(item);
                if should_remove {
                    // Update byte estimates
                    if let Some(size) = item.estimated_size {
                        self.estimated_total_bytes
                            .fetch_sub(size, Ordering::Relaxed);
                    }
                }
                !should_remove
            });

            let items_removed = original_len - queue_guard.len();
            removed_count += items_removed;
        }

        // Update total counter
        self.total_items.fetch_sub(removed_count, Ordering::Relaxed);

        if removed_count > 0 {
            debug!("Removed {} items matching predicate", removed_count);
        }

        removed_count
    }
}

/// Queue metrics for monitoring and debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueMetrics {
    pub total_items: usize,
    pub critical_priority_items: usize,
    pub high_priority_items: usize,
    pub medium_priority_items: usize,
    pub low_priority_items: usize,
    pub total_enqueued: u64,
    pub total_dequeued: u64,
    pub estimated_total_bytes: u64,
    pub is_paused: bool,
    pub max_size: usize,
    pub utilization_ratio: f64,
    pub age_seconds: u64,
}

/// Lightweight queue snapshot for serialization/IPC  
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueSnapshot {
    pub total_items: usize,
    pub critical_priority_items: usize,
    pub high_priority_items: usize,
    pub medium_priority_items: usize,
    pub low_priority_items: usize,
    pub estimated_total_bytes: u64,
    pub is_paused: bool,
    pub utilization_ratio: f64,
}

/// Generate a unique item ID
fn generate_item_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static ITEM_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
    ITEM_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tokio::time::{sleep, Duration as TokioDuration};

    #[tokio::test]
    async fn test_basic_queue_operations() {
        let queue = IndexingQueue::new(100);

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        // Test enqueue
        let item = QueueItem::high_priority(PathBuf::from("/test/file.rs"));
        assert!(queue.enqueue(item).await.unwrap());

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.len_for_priority(Priority::High).await, 1);

        // Test dequeue
        let dequeued = queue.dequeue().await.unwrap();
        assert_eq!(dequeued.file_path, Path::new("/test/file.rs"));
        assert_eq!(dequeued.priority, Priority::High);

        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let queue = IndexingQueue::unlimited();

        // Enqueue in reverse priority order
        let low_item = QueueItem::low_priority(PathBuf::from("/low.rs"));
        let med_item = QueueItem::medium_priority(PathBuf::from("/med.rs"));
        let high_item = QueueItem::high_priority(PathBuf::from("/high.rs"));

        queue.enqueue(low_item).await.unwrap();
        queue.enqueue(med_item).await.unwrap();
        queue.enqueue(high_item).await.unwrap();

        assert_eq!(queue.len(), 3);

        // Should dequeue in priority order
        let first = queue.dequeue().await.unwrap();
        assert_eq!(first.priority, Priority::High);

        let second = queue.dequeue().await.unwrap();
        assert_eq!(second.priority, Priority::Medium);

        let third = queue.dequeue().await.unwrap();
        assert_eq!(third.priority, Priority::Low);

        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn test_size_limit() {
        let queue = IndexingQueue::new(2);

        // Should accept up to limit
        assert!(queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/1.rs")))
            .await
            .unwrap());
        assert!(queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/2.rs")))
            .await
            .unwrap());

        // Should reject when at limit
        assert!(!queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/3.rs")))
            .await
            .unwrap());

        assert_eq!(queue.len(), 2);
    }

    #[tokio::test]
    async fn test_pause_resume() {
        let queue = IndexingQueue::unlimited();

        // Should work normally
        assert!(queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/test.rs")))
            .await
            .unwrap());
        assert!(queue.dequeue().await.is_some());

        // Pause queue
        queue.pause();
        assert!(queue.is_paused());

        // Should reject enqueue and return None for dequeue
        assert!(!queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/test2.rs")))
            .await
            .unwrap());
        assert!(queue.dequeue().await.is_none());

        // Resume and test
        queue.resume();
        assert!(!queue.is_paused());
        assert!(queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/test3.rs")))
            .await
            .unwrap());
        assert!(queue.dequeue().await.is_some());
    }

    #[tokio::test]
    async fn test_batch_operations() {
        let queue = IndexingQueue::unlimited();

        let items = vec![
            QueueItem::high_priority(PathBuf::from("/1.rs")),
            QueueItem::medium_priority(PathBuf::from("/2.rs")),
            QueueItem::low_priority(PathBuf::from("/3.rs")),
        ];

        let enqueued = queue.enqueue_batch(items).await.unwrap();
        assert_eq!(enqueued, 3);
        assert_eq!(queue.len(), 3);

        // Test clear
        queue.clear().await;
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn test_metrics() {
        let queue = IndexingQueue::new(100);

        let item_with_size =
            QueueItem::low_priority(PathBuf::from("/big.rs")).with_estimated_size(1024);

        queue.enqueue(item_with_size).await.unwrap();
        queue
            .enqueue(QueueItem::high_priority(PathBuf::from("/small.rs")))
            .await
            .unwrap();

        let metrics = queue.get_metrics().await;
        assert_eq!(metrics.total_items, 2);
        assert_eq!(metrics.high_priority_items, 1);
        assert_eq!(metrics.low_priority_items, 1);
        assert_eq!(metrics.estimated_total_bytes, 1024);
        assert_eq!(metrics.total_enqueued, 2);
        assert_eq!(metrics.total_dequeued, 0);
        assert!(metrics.utilization_ratio > 0.0);

        // Test dequeue updates metrics
        queue.dequeue().await.unwrap(); // Should dequeue high priority first
        let updated_metrics = queue.get_metrics().await;
        assert_eq!(updated_metrics.total_dequeued, 1);
        assert_eq!(updated_metrics.high_priority_items, 0);
    }

    #[tokio::test]
    async fn test_remove_matching() {
        let queue = IndexingQueue::unlimited();

        queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/keep.rs")))
            .await
            .unwrap();
        queue
            .enqueue(QueueItem::high_priority(PathBuf::from("/remove.tmp")))
            .await
            .unwrap();
        queue
            .enqueue(QueueItem::medium_priority(PathBuf::from("/keep2.rs")))
            .await
            .unwrap();

        assert_eq!(queue.len(), 3);

        // Remove items with .tmp extension
        let removed = queue
            .remove_matching(|item| {
                item.file_path.extension().and_then(|ext| ext.to_str()) == Some("tmp")
            })
            .await;

        assert_eq!(removed, 1);
        assert_eq!(queue.len(), 2);

        // Verify remaining items are correct
        let first = queue.dequeue().await.unwrap(); // Should be medium priority
        assert_eq!(first.priority, Priority::Medium);
        assert!(first.file_path.to_string_lossy().contains("keep2"));
    }

    #[tokio::test]
    async fn test_peek() {
        let queue = IndexingQueue::unlimited();

        let item = QueueItem::high_priority(PathBuf::from("/peek.rs"));
        queue.enqueue(item).await.unwrap();

        // Peek should return item without removing
        let peeked = queue.peek().await.unwrap();
        assert_eq!(peeked.file_path, Path::new("/peek.rs"));
        assert_eq!(queue.len(), 1);

        // Actual dequeue should return same item
        let dequeued = queue.dequeue().await.unwrap();
        assert_eq!(dequeued.id, peeked.id);
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        use std::sync::Arc;

        let queue = Arc::new(IndexingQueue::unlimited());
        let mut handles = Vec::new();

        // Spawn multiple tasks that enqueue items
        for i in 0..10 {
            let queue_clone = Arc::clone(&queue);
            let handle = tokio::spawn(async move {
                for j in 0..10 {
                    let path = format!("/test/{}_{}.rs", i, j);
                    let item = if j % 3 == 0 {
                        QueueItem::high_priority(PathBuf::from(path))
                    } else if j % 3 == 1 {
                        QueueItem::medium_priority(PathBuf::from(path))
                    } else {
                        QueueItem::low_priority(PathBuf::from(path))
                    };

                    queue_clone.enqueue(item).await.unwrap();

                    // Small delay to encourage interleaving
                    sleep(TokioDuration::from_millis(1)).await;
                }
            });
            handles.push(handle);
        }

        // Spawn tasks that dequeue items
        let dequeue_queue = Arc::clone(&queue);
        let dequeue_handle = tokio::spawn(async move {
            let mut dequeued_count = 0;
            while dequeued_count < 100 {
                if let Some(_item) = dequeue_queue.dequeue().await {
                    dequeued_count += 1;
                } else {
                    sleep(TokioDuration::from_millis(10)).await;
                }
            }
            dequeued_count
        });

        // Wait for all enqueue tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Wait for dequeue task
        let dequeued = dequeue_handle.await.unwrap();
        assert_eq!(dequeued, 100);
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn test_critical_priority_queue() {
        let queue = IndexingQueue::unlimited();

        // Enqueue items of all priority levels including critical
        let critical_item = QueueItem::critical_priority(PathBuf::from("/critical.rs"));
        let high_item = QueueItem::high_priority(PathBuf::from("/high.rs"));
        let medium_item = QueueItem::medium_priority(PathBuf::from("/medium.rs"));
        let low_item = QueueItem::low_priority(PathBuf::from("/low.rs"));

        queue.enqueue(low_item).await.unwrap();
        queue.enqueue(medium_item).await.unwrap();
        queue.enqueue(high_item).await.unwrap();
        queue.enqueue(critical_item).await.unwrap();

        assert_eq!(queue.len(), 4);
        assert_eq!(queue.len_for_priority(Priority::Critical).await, 1);

        // Critical should be dequeued first
        let first = queue.dequeue().await.unwrap();
        assert_eq!(first.priority, Priority::Critical);
        assert!(first.file_path.to_string_lossy().contains("critical"));
    }

    #[tokio::test]
    async fn test_queue_item_age_calculation() {
        let item = QueueItem::low_priority(PathBuf::from("/test.rs"));

        // Age should be very small immediately after creation
        let age = item.age();
        assert!(age.as_millis() < 100);

        // Wait and check age increases
        sleep(TokioDuration::from_millis(10)).await;
        let later_age = item.age();
        assert!(later_age > age);
    }

    #[tokio::test]
    async fn test_queue_item_builder_pattern() {
        let item = QueueItem::medium_priority(PathBuf::from("/test.rs"))
            .with_language_hint("rust".to_string())
            .with_estimated_size(2048)
            .with_metadata(serde_json::json!({"project": "test", "version": "1.0"}));

        assert_eq!(item.priority, Priority::Medium);
        assert_eq!(item.language_hint, Some("rust".to_string()));
        assert_eq!(item.estimated_size, Some(2048));
        assert!(item.metadata.is_object());
    }

    #[tokio::test]
    async fn test_priority_from_str() {
        assert_eq!(Priority::from_str("critical"), Some(Priority::Critical));
        assert_eq!(Priority::from_str("CRITICAL"), Some(Priority::Critical));
        assert_eq!(Priority::from_str("crit"), Some(Priority::Critical));
        assert_eq!(Priority::from_str("3"), Some(Priority::Critical));

        assert_eq!(Priority::from_str("high"), Some(Priority::High));
        assert_eq!(Priority::from_str("h"), Some(Priority::High));
        assert_eq!(Priority::from_str("2"), Some(Priority::High));

        assert_eq!(Priority::from_str("medium"), Some(Priority::Medium));
        assert_eq!(Priority::from_str("med"), Some(Priority::Medium));
        assert_eq!(Priority::from_str("1"), Some(Priority::Medium));

        assert_eq!(Priority::from_str("low"), Some(Priority::Low));
        assert_eq!(Priority::from_str("0"), Some(Priority::Low));

        assert_eq!(Priority::from_str("invalid"), None);
    }

    #[tokio::test]
    async fn test_memory_tracking() {
        let queue = IndexingQueue::unlimited();

        // Enqueue items with size estimates
        let item1 = QueueItem::high_priority(PathBuf::from("/file1.rs")).with_estimated_size(1024);
        let item2 = QueueItem::low_priority(PathBuf::from("/file2.rs")).with_estimated_size(2048);

        queue.enqueue(item1).await.unwrap();
        queue.enqueue(item2).await.unwrap();

        let metrics = queue.get_metrics().await;
        assert_eq!(metrics.estimated_total_bytes, 3072);

        // Dequeue and verify memory tracking updates
        queue.dequeue().await.unwrap(); // High priority first
        let updated_metrics = queue.get_metrics().await;
        assert_eq!(updated_metrics.estimated_total_bytes, 2048);

        // Clear and verify memory is reset
        queue.clear().await;
        let final_metrics = queue.get_metrics().await;
        assert_eq!(final_metrics.estimated_total_bytes, 0);
    }

    #[tokio::test]
    async fn test_queue_clear_by_priority() {
        let queue = IndexingQueue::unlimited();

        // Enqueue items across priorities
        queue
            .enqueue(QueueItem::critical_priority(PathBuf::from("/c.rs")))
            .await
            .unwrap();
        queue
            .enqueue(QueueItem::high_priority(PathBuf::from("/h.rs")))
            .await
            .unwrap();
        queue
            .enqueue(QueueItem::medium_priority(PathBuf::from("/m.rs")))
            .await
            .unwrap();
        queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/l.rs")))
            .await
            .unwrap();

        assert_eq!(queue.len(), 4);

        // Clear only high priority
        queue.clear_priority(Priority::High).await;
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.len_for_priority(Priority::High).await, 0);

        // Other priorities should remain
        assert_eq!(queue.len_for_priority(Priority::Critical).await, 1);
        assert_eq!(queue.len_for_priority(Priority::Medium).await, 1);
        assert_eq!(queue.len_for_priority(Priority::Low).await, 1);
    }

    #[tokio::test]
    async fn test_stress_high_volume_operations() {
        let queue = IndexingQueue::unlimited();
        const ITEM_COUNT: usize = 1000;

        // Enqueue many items
        let mut tasks = Vec::new();
        for i in 0..ITEM_COUNT {
            let queue_clone = Arc::new(queue.clone());
            let task = tokio::spawn(async move {
                let path = format!("/stress/file_{}.rs", i);
                let priority = match i % 4 {
                    0 => Priority::Critical,
                    1 => Priority::High,
                    2 => Priority::Medium,
                    _ => Priority::Low,
                };
                let item = QueueItem::new(PathBuf::from(path), priority);
                queue_clone.enqueue(item).await.unwrap();
            });
            tasks.push(task);
        }

        // Wait for all enqueues to complete
        for task in tasks {
            task.await.unwrap();
        }

        assert_eq!(queue.len(), ITEM_COUNT);

        // Dequeue all items and verify priority ordering is maintained
        let mut previous_priority = Priority::Critical;
        let mut dequeued_count = 0;

        while let Some(item) = queue.dequeue().await {
            // Priority should be <= previous priority (same or lower priority value)
            // Critical=3 should come first, then High=2, Medium=1, Low=0
            assert!(item.priority.as_u8() <= previous_priority.as_u8());
            previous_priority = item.priority;
            dequeued_count += 1;
        }

        assert_eq!(dequeued_count, ITEM_COUNT);
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn test_queue_snapshot_serialization() {
        let queue = IndexingQueue::new(50);

        // Add some items
        queue
            .enqueue(QueueItem::high_priority(PathBuf::from("/h.rs")))
            .await
            .unwrap();
        queue
            .enqueue(QueueItem::low_priority(PathBuf::from("/l.rs")))
            .await
            .unwrap();

        let snapshot = queue.get_snapshot().await;

        // Test serialization
        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: QueueSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.total_items, 2);
        assert_eq!(deserialized.high_priority_items, 1);
        assert_eq!(deserialized.low_priority_items, 1);
        assert!(!deserialized.is_paused);
        assert!(deserialized.utilization_ratio > 0.0);
    }

    #[tokio::test]
    async fn test_edge_case_empty_operations() {
        let queue = IndexingQueue::unlimited();

        // Operations on empty queue
        assert!(queue.dequeue().await.is_none());
        assert!(queue.peek().await.is_none());

        // Clear empty queue should not panic
        queue.clear().await;
        queue.clear_priority(Priority::High).await;

        // Remove matching on empty queue
        let removed = queue.remove_matching(|_| true).await;
        assert_eq!(removed, 0);

        let metrics = queue.get_metrics().await;
        assert_eq!(metrics.total_items, 0);
        assert_eq!(metrics.estimated_total_bytes, 0);
    }

    #[tokio::test]
    async fn test_batch_enqueue_with_size_limit() {
        let queue = IndexingQueue::new(3);

        let items = vec![
            QueueItem::high_priority(PathBuf::from("/1.rs")),
            QueueItem::medium_priority(PathBuf::from("/2.rs")),
            QueueItem::low_priority(PathBuf::from("/3.rs")),
            QueueItem::high_priority(PathBuf::from("/4.rs")), // Should be rejected
            QueueItem::low_priority(PathBuf::from("/5.rs")),  // Should be rejected
        ];

        let enqueued_count = queue.enqueue_batch(items).await.unwrap();
        assert_eq!(enqueued_count, 3); // Only first 3 should be accepted
        assert_eq!(queue.len(), 3);

        let metrics = queue.get_metrics().await;
        assert_eq!(metrics.utilization_ratio, 1.0); // 100% utilized
    }

    #[tokio::test]
    async fn test_queue_item_unique_ids() {
        let item1 = QueueItem::new(PathBuf::from("/test1.rs"), Priority::High);
        let item2 = QueueItem::new(PathBuf::from("/test2.rs"), Priority::High);

        // IDs should be unique
        assert_ne!(item1.id, item2.id);

        // IDs should be sequential
        assert!(item2.id > item1.id);
    }

    #[tokio::test]
    async fn test_pause_during_operations() {
        let queue = Arc::new(IndexingQueue::unlimited());

        // Start enqueueing items
        let enqueue_handle = {
            let queue = Arc::clone(&queue);
            tokio::spawn(async move {
                let mut enqueued = 0;
                for i in 0..100 {
                    let item = QueueItem::low_priority(PathBuf::from(format!("/file_{}.rs", i)));
                    if queue.enqueue(item).await.unwrap() {
                        enqueued += 1;
                    }
                    sleep(TokioDuration::from_millis(1)).await;
                }
                enqueued
            })
        };

        // Pause after some items are enqueued
        sleep(TokioDuration::from_millis(20)).await;
        queue.pause();

        let enqueued_count = enqueue_handle.await.unwrap();

        // Should have enqueued some items before pause
        assert!(enqueued_count > 0);
        assert!(enqueued_count < 100); // But not all due to pause

        // After pause, dequeue should return None
        assert!(queue.dequeue().await.is_none());

        // Resume and verify we can dequeue
        queue.resume();
        assert!(queue.dequeue().await.is_some());
    }
}
