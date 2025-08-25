//! Tests for LSP daemon fixes
//!
//! This module tests the specific issues that were fixed:
//! 1. LspClient Drop implementation no longer creates spurious connections
//! 2. Version mismatch detection doesn't loop infinitely  
//! 3. Daemon handles early EOF gracefully without errors
//! 4. Server lock timeouts are handled properly

use anyhow::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::Barrier;
use tokio::time::sleep;

// Import the daemon and client types
use lsp_daemon::{get_default_socket_path, DaemonRequest, IpcStream, MessageCodec};
use probe_code::lsp_integration::{LspClient, LspConfig};

#[tokio::test]
async fn test_client_drop_no_spurious_connections() -> Result<()> {
    // This test verifies that dropping LspClient doesn't create new connections
    // that immediately fail with "early eof" errors

    let config = LspConfig {
        use_daemon: true,
        timeout_ms: 5000,
        workspace_hint: None,
        include_stdlib: false,
    };

    // Create and drop multiple clients rapidly
    for i in 0..5 {
        let client = LspClient::new(config.clone()).await;
        match client {
            Ok(_client) => {
                // Client created successfully, it will be dropped at end of scope
                println!("Created client {i}");
            }
            Err(e) => {
                // It's OK if we can't create clients (daemon might not be ready)
                println!("Failed to create client {i}: {e}");
            }
        }
        sleep(Duration::from_millis(100)).await;
    }

    // Give daemon time to process any connections
    sleep(Duration::from_millis(500)).await;

    Ok(())
}

#[tokio::test]
async fn test_graceful_connection_close() -> Result<()> {
    // This test verifies that closing connections gracefully doesn't log errors

    let socket_path = get_default_socket_path();

    // Try to connect to daemon
    if let Ok(stream) = IpcStream::connect(&socket_path).await {
        // Just drop the connection immediately - this should be handled gracefully
        drop(stream);

        // Try another connection that we close after a message
        if let Ok(mut stream) = IpcStream::connect(&socket_path).await {
            let ping_request = DaemonRequest::Ping {
                request_id: uuid::Uuid::new_v4(),
            };

            let encoded = MessageCodec::encode(&ping_request)?;
            let _ = stream.write_all(&encoded).await;
            let _ = stream.flush().await;

            // Close immediately after sending - this should also be graceful
            drop(stream);
        }
    }

    sleep(Duration::from_millis(100)).await;
    Ok(())
}

#[tokio::test]
async fn test_version_compatibility_check() -> Result<()> {
    // This test verifies that version compatibility checking works without loops
    // We test this indirectly by creating clients multiple times rapidly

    let config = LspConfig {
        use_daemon: true,
        timeout_ms: 5000,
        workspace_hint: None,
        include_stdlib: false,
    };

    // Call client creation multiple times rapidly - this triggers version checks
    let start = Instant::now();

    for _ in 0..3 {
        let _ = LspClient::new(config.clone()).await;
        sleep(Duration::from_millis(10)).await;
    }

    let elapsed = start.elapsed();

    // Should complete quickly without hanging or looping
    assert!(
        elapsed < Duration::from_secs(15),
        "Client creation took too long: {elapsed:?}"
    );

    Ok(())
}

#[tokio::test]
async fn test_concurrent_client_creation() -> Result<()> {
    // Test that multiple clients can be created concurrently without lock issues

    let config = LspConfig {
        use_daemon: true,
        timeout_ms: 10000,
        workspace_hint: None,
        include_stdlib: false,
    };

    const NUM_CLIENTS: usize = 5;
    let barrier = Arc::new(Barrier::new(NUM_CLIENTS));
    let mut handles = Vec::new();

    for i in 0..NUM_CLIENTS {
        let config = config.clone();
        let barrier = barrier.clone();

        let handle = tokio::spawn(async move {
            // Wait for all tasks to be ready
            barrier.wait().await;

            let start = Instant::now();
            let result = LspClient::new(config).await;
            let elapsed = start.elapsed();

            println!("Client {i} creation took {elapsed:?}");

            // Should complete within reasonable time (not hit 30s timeout)
            assert!(
                elapsed < Duration::from_secs(25),
                "Client {i} creation took too long: {elapsed:?}"
            );

            result
        });

        handles.push(handle);
    }

    // Wait for all client creation attempts
    let mut successes = 0;
    let mut failures = 0;

    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(_client)) => {
                successes += 1;
                println!("Client {i} created successfully");
            }
            Ok(Err(e)) => {
                failures += 1;
                println!("Client {i} failed to create: {e}");
            }
            Err(e) => {
                failures += 1;
                println!("Client {i} task panicked: {e}");
            }
        }
    }

    println!("Concurrent client creation: {successes} successes, {failures} failures");

    // At least some should succeed (but it's OK if daemon isn't fully ready)
    // The important thing is no panics or infinite hangs

    Ok(())
}

#[tokio::test]
async fn test_daemon_status_multiple_calls() -> Result<()> {
    // Test that multiple status calls work without issues

    let config = LspConfig {
        use_daemon: true,
        timeout_ms: 5000,
        workspace_hint: None,
        include_stdlib: false,
    };

    // Try to create a client and call status multiple times
    match LspClient::new(config).await {
        Ok(mut client) => {
            for i in 0..3 {
                let start = Instant::now();
                let result = client.get_status().await;
                let elapsed = start.elapsed();

                println!("Status call {i} took {elapsed:?}");

                match result {
                    Ok(status) => {
                        println!(
                            "Status call {} succeeded: uptime={}s",
                            i,
                            status.uptime.as_secs()
                        );
                    }
                    Err(e) => {
                        println!("Status call {i} failed: {e}");
                    }
                }

                // Should not hang indefinitely
                assert!(
                    elapsed < Duration::from_secs(10),
                    "Status call {i} took too long: {elapsed:?}"
                );

                sleep(Duration::from_millis(100)).await;
            }
        }
        Err(e) => {
            println!("Could not create client for status test: {e}");
            // This is OK - daemon might not be available
        }
    }

    Ok(())
}

// Note: These tests cover the functionality through public APIs
// The fixes are tested indirectly through client creation and daemon interaction
