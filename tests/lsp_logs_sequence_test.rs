use anyhow::Result;
use lsp_daemon::{LogBuffer, LogEntry, LogLevel};
use std::time::Duration;
use tokio::time::sleep;

/// Integration test for sequence-based logging functionality
/// This tests the critical reconnection scenario mentioned in the issue
#[tokio::test]
async fn test_logs_follow_reconnection_scenario() -> Result<()> {
    // Create a log buffer to simulate daemon behavior
    let log_buffer = LogBuffer::new();

    // Add some initial logs
    for i in 0..5 {
        let entry = LogEntry {
            sequence: 0, // Will be set by push
            timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: format!("Initial log entry {i}"),
            file: None,
            line: None,
        };
        log_buffer.push(entry);
    }

    // First connection - simulate getting logs
    let logs1 = log_buffer.get_last(10);
    assert_eq!(logs1.len(), 5);

    // Track the last sequence we saw
    let last_sequence_from_first_connection = logs1.iter().map(|e| e.sequence).max().unwrap_or(0);
    assert_eq!(last_sequence_from_first_connection, 4); // Sequences 0-4

    // Simulate some activity that generates new logs (this would happen between connections)
    for i in 5..10 {
        let entry = LogEntry {
            sequence: 0, // Will be set by push
            timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: format!("New log entry {i}"),
            file: None,
            line: None,
        };
        log_buffer.push(entry);
    }

    // Second connection - this is where the issue occurred
    // Using get_since_sequence, we should only get the new logs
    let new_logs = log_buffer.get_since_sequence(last_sequence_from_first_connection, 100);

    // Verify we get only the new logs
    assert_eq!(new_logs.len(), 5); // Should get sequences 5-9
    for (i, log) in new_logs.iter().enumerate() {
        let expected_sequence = last_sequence_from_first_connection + 1 + i as u64;
        assert_eq!(log.sequence, expected_sequence);
        assert!(log.message.contains(&format!("New log entry {}", 5 + i)));
    }

    Ok(())
}

#[tokio::test]
async fn test_rapid_log_generation_with_follow_mode() -> Result<()> {
    let log_buffer = LogBuffer::new();

    // Add initial logs
    for i in 0..10 {
        let entry = LogEntry {
            sequence: 0, // Will be set by push
            timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: format!("Initial entry {i}"),
            file: None,
            line: None,
        };
        log_buffer.push(entry);
    }

    let initial_logs = log_buffer.get_last(10);
    let mut last_seen_sequence = initial_logs.iter().map(|e| e.sequence).max().unwrap_or(0);

    // Simulate rapid log generation in the background
    let log_buffer_clone = log_buffer.clone();
    let generate_logs_task = tokio::spawn(async move {
        for i in 10..110 {
            let entry = LogEntry {
                sequence: 0, // Will be set by push
                timestamp: format!("2024-01-01 12:00:{:02}.000 UTC", i % 60),
                level: LogLevel::Info,
                target: "test".to_string(),
                message: format!("Rapid entry {i}"),
                file: None,
                line: None,
            };
            log_buffer_clone.push(entry);

            // Small delay to simulate real logging
            sleep(Duration::from_millis(1)).await;
        }
    });

    // Simulate follow mode polling
    let mut total_new_logs = 0;
    for _ in 0..20 {
        sleep(Duration::from_millis(10)).await;

        let new_logs = log_buffer.get_since_sequence(last_seen_sequence, 50);
        total_new_logs += new_logs.len();

        // Update last seen sequence
        for log in &new_logs {
            if log.sequence > last_seen_sequence {
                last_seen_sequence = log.sequence;
            }
        }
    }

    // Wait for log generation to complete
    generate_logs_task.await?;

    // Get any remaining logs
    let final_logs = log_buffer.get_since_sequence(last_seen_sequence, 100);
    total_new_logs += final_logs.len();

    // Update last seen sequence
    for log in &final_logs {
        if log.sequence > last_seen_sequence {
            last_seen_sequence = log.sequence;
        }
    }

    // Verify we captured all logs
    assert_eq!(total_new_logs, 100, "Should have captured all 100 new logs");
    assert_eq!(
        last_seen_sequence, 109,
        "Should have seen all sequences up to 109"
    );

    Ok(())
}

#[tokio::test]
async fn test_sequence_based_deduplication() -> Result<()> {
    let log_buffer = LogBuffer::new();

    // Add some logs
    for i in 0..5 {
        let entry = LogEntry {
            sequence: 0, // Will be set by push
            timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: format!("Entry {i}"),
            file: None,
            line: None,
        };
        log_buffer.push(entry);
    }

    // Simulate multiple follow mode requests with the same sequence
    let last_sequence = 2;

    // Multiple requests should return the same results
    let logs1 = log_buffer.get_since_sequence(last_sequence, 10);
    let logs2 = log_buffer.get_since_sequence(last_sequence, 10);
    let logs3 = log_buffer.get_since_sequence(last_sequence, 10);

    assert_eq!(logs1.len(), logs2.len());
    assert_eq!(logs2.len(), logs3.len());
    assert_eq!(logs1.len(), 2); // Should get sequences 3 and 4

    // Verify they're identical
    for i in 0..logs1.len() {
        assert_eq!(logs1[i].sequence, logs2[i].sequence);
        assert_eq!(logs2[i].sequence, logs3[i].sequence);
        assert_eq!(logs1[i].message, logs2[i].message);
    }

    Ok(())
}

#[tokio::test]
async fn test_concurrent_connections_sequence_consistency() -> Result<()> {
    let log_buffer = LogBuffer::new();

    // Pre-fill with some logs
    for i in 0..10 {
        let entry = LogEntry {
            sequence: 0,
            timestamp: format!("2024-01-01 12:00:{i:02}.000 UTC"),
            level: LogLevel::Info,
            target: "test".to_string(),
            message: format!("Prefill entry {i}"),
            file: None,
            line: None,
        };
        log_buffer.push(entry);
    }

    // Spawn multiple concurrent "connections" that simulate follow mode
    let mut handles = Vec::new();

    for client_id in 0..5 {
        let buffer_clone = log_buffer.clone();
        let handle = tokio::spawn(async move {
            let mut last_seen = 9; // Start after prefilled logs
            let mut seen_logs = Vec::new();

            for _ in 0..10 {
                sleep(Duration::from_millis(5)).await;
                let new_logs = buffer_clone.get_since_sequence(last_seen, 20);

                for log in new_logs {
                    if log.sequence > last_seen {
                        seen_logs.push(log.sequence);
                        last_seen = log.sequence;
                    }
                }
            }

            (client_id, seen_logs)
        });
        handles.push(handle);
    }

    // Generate logs concurrently
    let log_generator = tokio::spawn({
        let buffer_clone = log_buffer.clone();
        async move {
            for i in 10..50 {
                let entry = LogEntry {
                    sequence: 0,
                    timestamp: format!("2024-01-01 12:01:{:02}.000 UTC", i % 60),
                    level: LogLevel::Info,
                    target: "generator".to_string(),
                    message: format!("Generated entry {i}"),
                    file: None,
                    line: None,
                };
                buffer_clone.push(entry);
                sleep(Duration::from_millis(2)).await;
            }
        }
    });

    // Wait for all tasks to complete
    let mut all_client_logs = Vec::new();
    for handle in handles {
        let (client_id, logs) = handle.await?;
        all_client_logs.push((client_id, logs));
    }

    log_generator.await?;

    // Verify that all clients saw consistent sequence numbers
    // Each client may have seen different subsets, but sequences should be consistent
    for (client_id, logs) in &all_client_logs {
        // Check that sequences are monotonic for each client
        for window in logs.windows(2) {
            assert!(
                window[1] > window[0],
                "Client {} saw non-monotonic sequences: {} -> {}",
                client_id,
                window[0],
                window[1]
            );
        }
    }

    // Verify no client saw duplicate sequences
    for (client_id, logs) in &all_client_logs {
        let mut unique_logs = logs.clone();
        unique_logs.sort();
        unique_logs.dedup();
        assert_eq!(
            logs.len(),
            unique_logs.len(),
            "Client {client_id} saw duplicate sequences"
        );
    }

    Ok(())
}

#[test]
fn test_sequence_wraparound_behavior() {
    // Test what happens when sequence numbers get very large
    // This is more of a theoretical test since u64 is huge
    use std::sync::atomic::{AtomicU64, Ordering};

    let counter = AtomicU64::new(u64::MAX - 5);

    // Test that we can still increment near the max
    for i in 0..10 {
        let seq = counter.fetch_add(1, Ordering::SeqCst);
        if i < 5 {
            assert!(seq < u64::MAX);
        } else {
            // After wrapping, sequences continue (though this is unlikely in practice)
            // The important thing is that the atomic operation doesn't panic
        }
    }
}
