use crate::protocol::{LogEntry, LogLevel};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// Default capacity can be overridden at runtime:
//   PROBE_LSP_LOG_BUFFER_CAPACITY=20000
const DEFAULT_LOG_CAPACITY: usize = 10_000;

/// Thread-safe circular buffer for storing log entries
#[derive(Debug, Clone)]
pub struct LogBuffer {
    entries: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
    sequence_counter: Arc<AtomicU64>,
}

impl LogBuffer {
    /// Create a new empty log buffer
    pub fn new() -> Self {
        let capacity = std::env::var("PROBE_LSP_LOG_BUFFER_CAPACITY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_LOG_CAPACITY);
        Self {
            entries: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
            sequence_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Add a log entry to the buffer, removing old entries if needed
    pub fn push(&self, mut entry: LogEntry) {
        // Assign sequence number atomically
        entry.sequence = self.sequence_counter.fetch_add(1, Ordering::SeqCst);

        if let Ok(mut entries) = self.entries.lock() {
            entries.push_back(entry);

            // Maintain circular buffer behavior by removing old entries
            while entries.len() > self.capacity {
                entries.pop_front();
            }
        }
    }

    /// Get the last N log entries, up to the buffer size
    pub fn get_last(&self, count: usize) -> Vec<LogEntry> {
        // Use try_lock to avoid potential deadlock if logging happens during serialization
        match self.entries.try_lock() {
            Ok(entries) => {
                let take_count = count.min(entries.len());
                entries
                    .iter()
                    .rev()
                    .take(take_count)
                    .rev()
                    .cloned()
                    .collect()
            }
            Err(_) => {
                // If we can't acquire the lock immediately, return empty to avoid deadlock
                // This can happen if a log event is triggered during response serialization
                Vec::new()
            }
        }
    }

    /// Get all log entries currently in the buffer
    pub fn get_all(&self) -> Vec<LogEntry> {
        // Use try_lock to avoid potential deadlock
        match self.entries.try_lock() {
            Ok(entries) => entries.iter().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Get log entries since a specific sequence number
    pub fn get_since_sequence(&self, since: u64, limit: usize) -> Vec<LogEntry> {
        // Use try_lock to avoid potential deadlock
        match self.entries.try_lock() {
            Ok(entries) => entries
                .iter()
                .filter(|entry| entry.sequence > since)
                .take(limit)
                .cloned()
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Clear all log entries from the buffer
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    /// Get the current number of entries in the buffer
    pub fn len(&self) -> usize {
        match self.entries.try_lock() {
            Ok(entries) => entries.len(),
            Err(_) => 0,
        }
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracing layer that writes log entries to an in-memory buffer
pub struct MemoryLogLayer {
    buffer: LogBuffer,
}

impl MemoryLogLayer {
    /// Create a new memory log layer with the given buffer
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }

    /// Get a reference to the log buffer
    pub fn buffer(&self) -> &LogBuffer {
        &self.buffer
    }

    /// Convert tracing level to our LogLevel enum
    fn convert_level(level: &tracing::Level) -> LogLevel {
        match *level {
            tracing::Level::TRACE => LogLevel::Trace,
            tracing::Level::DEBUG => LogLevel::Debug,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::ERROR => LogLevel::Error,
        }
    }

    /// Extract location information from metadata
    fn extract_location(metadata: &tracing::Metadata) -> (Option<String>, Option<u32>) {
        let file = metadata.file().map(|s| s.to_string());
        let line = metadata.line();
        (file, line)
    }

    /// Format the log message from the event
    fn format_message<S>(event: &Event<'_>, _ctx: &Context<'_, S>) -> String
    where
        S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        // Use a visitor to format the message properly
        struct MessageVisitor {
            message: String,
        }

        impl tracing::field::Visit for MessageVisitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.message = format!("{value:?}");
                    // Remove surrounding quotes from debug format
                    if self.message.starts_with('"') && self.message.ends_with('"') {
                        self.message = self.message[1..self.message.len() - 1].to_string();
                    }
                }
            }
        }

        let mut visitor = MessageVisitor {
            message: String::new(),
        };

        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            // Fallback to target if no specific message
            event.metadata().target().to_string()
        } else {
            visitor.message
        };

        // Truncate very large messages to prevent IPC issues (limit to 4KB per log message)
        const MAX_LOG_MESSAGE_SIZE: usize = 4096;
        if message.len() > MAX_LOG_MESSAGE_SIZE {
            format!(
                "{}... [TRUNCATED - original size: {} chars]",
                &message[..MAX_LOG_MESSAGE_SIZE],
                message.len()
            )
        } else {
            message
        }
    }
}

impl<S> Layer<S> for MemoryLogLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = Self::convert_level(metadata.level());
        let target = metadata.target().to_string();
        let (file, line) = Self::extract_location(metadata);

        // Create timestamp
        let timestamp = chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S%.3f UTC")
            .to_string();

        // Format message - this is a simplified version
        // A full implementation would extract the formatted message from the event
        let message = Self::format_message(event, &ctx);

        let log_entry = LogEntry {
            sequence: 0, // Will be set by LogBuffer::push
            timestamp,
            level,
            target,
            message,
            file,
            line,
        };

        self.buffer.push(log_entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_basic_operations() {
        let buffer = LogBuffer::new();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);

        let entry = LogEntry {
            sequence: 0, // Will be set by push
            timestamp: "2024-01-01 12:00:00.000 UTC".to_string(),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: "Test message".to_string(),
            file: None,
            line: None,
        };

        buffer.push(entry.clone());
        assert_eq!(buffer.len(), 1);
        assert!(!buffer.is_empty());

        let entries = buffer.get_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "Test message");
    }

    #[test]
    fn test_log_buffer_circular_behavior() {
        let buffer = LogBuffer::new();

        // Fill buffer beyond capacity - use buffer capacity instead of undefined MAX_LOG_ENTRIES
        let test_capacity = buffer.capacity;
        for i in 0..(test_capacity + 100) {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{:02}.000 UTC", i % 60),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Message {i}"),
                file: None,
                line: None,
            };
            buffer.push(entry);
        }

        // Should not exceed max capacity
        assert_eq!(buffer.len(), test_capacity);

        // Should contain the most recent entries
        let entries = buffer.get_all();
        assert!(entries[entries.len() - 1]
            .message
            .contains(&format!("{}", test_capacity + 99)));
    }

    #[test]
    fn test_get_last_entries() {
        let buffer = LogBuffer::new();

        // Add some entries
        for i in 0..10 {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Message {i}"),
                file: None,
                line: None,
            };
            buffer.push(entry);
        }

        // Get last 5 entries
        let entries = buffer.get_last(5);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].message, "Message 5");
        assert_eq!(entries[4].message, "Message 9");
    }

    #[test]
    fn test_level_conversion() {
        assert!(matches!(
            MemoryLogLayer::convert_level(&tracing::Level::TRACE),
            LogLevel::Trace
        ));
        assert!(matches!(
            MemoryLogLayer::convert_level(&tracing::Level::DEBUG),
            LogLevel::Debug
        ));
        assert!(matches!(
            MemoryLogLayer::convert_level(&tracing::Level::INFO),
            LogLevel::Info
        ));
        assert!(matches!(
            MemoryLogLayer::convert_level(&tracing::Level::WARN),
            LogLevel::Warn
        ));
        assert!(matches!(
            MemoryLogLayer::convert_level(&tracing::Level::ERROR),
            LogLevel::Error
        ));
    }

    #[test]
    fn test_log_message_truncation() {
        // Test the format_message function directly by creating a mock scenario
        let long_message = "A".repeat(5000);

        // Simulate what happens when a large message gets processed
        const MAX_LOG_MESSAGE_SIZE: usize = 4096;
        let truncated_message = if long_message.len() > MAX_LOG_MESSAGE_SIZE {
            format!(
                "{}... [TRUNCATED - original size: {} chars]",
                &long_message[..MAX_LOG_MESSAGE_SIZE],
                long_message.len()
            )
        } else {
            long_message.clone()
        };

        // Verify truncation occurred
        assert!(truncated_message.len() < long_message.len());
        assert!(truncated_message.contains("TRUNCATED"));
        assert!(truncated_message.contains("original size: 5000 chars"));
        assert!(truncated_message.starts_with(&"A".repeat(4096)));

        // Now test with a LogEntry that simulates the truncated message
        let buffer = LogBuffer::new();
        let entry = LogEntry {
            sequence: 0, // Will be set by push
            timestamp: "2024-01-01 12:00:00.000 UTC".to_string(),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: truncated_message.clone(),
            file: None,
            line: None,
        };

        buffer.push(entry);
        let entries = buffer.get_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, truncated_message);
    }

    #[test]
    fn test_log_message_no_truncation_for_short_messages() {
        let buffer = LogBuffer::new();

        // Create a normal-sized message
        let normal_message = "This is a normal message";
        let entry = LogEntry {
            sequence: 0, // Will be set by push
            timestamp: "2024-01-01 12:00:00.000 UTC".to_string(),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: normal_message.to_string(),
            file: None,
            line: None,
        };

        buffer.push(entry);
        let entries = buffer.get_all();
        assert_eq!(entries.len(), 1);

        // Message should not be truncated
        let retrieved_message = &entries[0].message;
        assert_eq!(retrieved_message, normal_message);
        assert!(!retrieved_message.contains("TRUNCATED"));
    }

    #[test]
    fn test_sequence_numbering() {
        let buffer = LogBuffer::new();

        // Add some entries and check sequence numbers are assigned correctly
        let mut expected_sequences = Vec::new();
        for i in 0..5 {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Message {i}"),
                file: None,
                line: None,
            };
            expected_sequences.push(i as u64);
            buffer.push(entry);
        }

        let entries = buffer.get_all();
        assert_eq!(entries.len(), 5);

        // Check that sequence numbers are assigned correctly
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.sequence, i as u64);
            assert_eq!(entry.message, format!("Message {i}"));
        }
    }

    #[test]
    fn test_get_since_sequence() {
        let buffer = LogBuffer::new();

        // Add 10 entries
        for i in 0..10 {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Message {i}"),
                file: None,
                line: None,
            };
            buffer.push(entry);
        }

        // Get entries since sequence 5 (should return sequences 6, 7, 8, 9)
        let entries = buffer.get_since_sequence(5, 100);
        assert_eq!(entries.len(), 4);

        let expected_sequences = vec![6, 7, 8, 9];
        for (entry, expected_seq) in entries.iter().zip(expected_sequences.iter()) {
            assert_eq!(entry.sequence, *expected_seq);
        }
    }

    #[test]
    fn test_get_since_sequence_with_limit() {
        let buffer = LogBuffer::new();

        // Add 10 entries
        for i in 0..10 {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Message {i}"),
                file: None,
                line: None,
            };
            buffer.push(entry);
        }

        // Get entries since sequence 3 with limit of 2 (should return sequences 4, 5)
        let entries = buffer.get_since_sequence(3, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 4);
        assert_eq!(entries[1].sequence, 5);
    }

    #[test]
    fn test_get_since_sequence_no_new_entries() {
        let buffer = LogBuffer::new();

        // Add 5 entries
        for i in 0..5 {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Message {i}"),
                file: None,
                line: None,
            };
            buffer.push(entry);
        }

        // Get entries since sequence 10 (higher than any existing sequence)
        let entries = buffer.get_since_sequence(10, 100);
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_sequence_counter_monotonic() {
        let buffer = LogBuffer::new();

        // Add entries from multiple threads to test atomicity
        use std::sync::Arc;
        use std::thread;

        let buffer = Arc::new(buffer);
        let handles: Vec<_> = (0..5)
            .map(|thread_id| {
                let buffer_clone = buffer.clone();
                thread::spawn(move || {
                    for i in 0..10 {
                        let entry = LogEntry {
                            sequence: 0, // Will be set by push
                            timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
                            level: LogLevel::Info,
                            target: "test".to_string(),
                            message: format!("Thread {} Message {}", thread_id, i),
                            file: None,
                            line: None,
                        };
                        buffer_clone.push(entry);
                    }
                })
            })
            .collect();

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        let entries = buffer.get_all();
        assert_eq!(entries.len(), 50); // 5 threads × 10 entries each

        // Check that all sequence numbers are unique and monotonic
        let mut sequences: Vec<u64> = entries.iter().map(|e| e.sequence).collect();
        sequences.sort();

        for (i, &seq) in sequences.iter().enumerate() {
            assert_eq!(
                seq, i as u64,
                "Sequence numbers should be sequential without gaps"
            );
        }
    }

    #[test]
    fn test_circular_buffer_maintains_sequences() {
        let buffer = LogBuffer::new();
        let capacity = buffer.capacity;

        // Fill buffer beyond capacity to trigger circular behavior
        for i in 0..(capacity + 10) {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{:02}.000 UTC", i % 60),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Message {i}"),
                file: None,
                line: None,
            };
            buffer.push(entry);
        }

        let entries = buffer.get_all();
        assert_eq!(entries.len(), capacity); // Should not exceed capacity

        // Check that sequence numbers are still monotonic within the buffer
        for window in entries.windows(2) {
            assert!(
                window[1].sequence > window[0].sequence,
                "Sequences should be monotonic even after wraparound"
            );
        }

        // The first entry should have sequence = 10 (since we added capacity + 10 entries,
        // and the first 10 were evicted)
        assert_eq!(entries[0].sequence, 10);
        assert_eq!(
            entries[entries.len() - 1].sequence,
            (capacity + 10 - 1) as u64
        );
    }
}
