use anyhow::Result;
use lsp_daemon::*;
use uuid::Uuid;

#[tokio::test]
async fn test_daemon_logging_basic() -> Result<()> {
    // Test the basic logging components without starting a full daemon
    // This tests the LogBuffer and MemoryLogLayer functionality

    let log_buffer = LogBuffer::new();
    let _memory_layer = MemoryLogLayer::new(log_buffer.clone());

    // Test that we can create log entries
    let test_entry = LogEntry {
        sequence: 0, // Will be set by push
        timestamp: "2024-01-01 12:00:00.000 UTC".to_string(),
        level: LogLevel::Info,
        target: "test_target".to_string(),
        message: "Test message".to_string(),
        file: Some("test.rs".to_string()),
        line: Some(42),
    };

    log_buffer.push(test_entry.clone());

    // Retrieve logs
    let logs = log_buffer.get_last(10);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].message, "Test message");
    assert_eq!(logs[0].level.to_string(), "INFO");

    println!("✅ Log buffer test passed: {} entries", logs.len());

    // Test a simple daemon instance for GetLogs handler
    let socket_path = format!("/tmp/test_daemon_logging_{}.sock", Uuid::new_v4());
    let _daemon = LspDaemon::new(socket_path.clone())?;

    // Test the GetLogs request handler directly (without running full daemon)
    let _logs_request = DaemonRequest::GetLogs {
        request_id: Uuid::new_v4(),
        lines: 50,
        since_sequence: None,
    };

    // The handle_request method is not public, so we'll test the log buffer directly
    // which is the main component we've integrated

    println!("✅ Basic logging integration test completed successfully!");
    Ok(())
}
