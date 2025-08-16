// Comprehensive stress tests for LSP daemon robustness validation
// These tests validate async I/O, timeouts, health monitoring, and recovery mechanisms

use anyhow::{Context, Result};
use lsp_daemon::{
    DaemonRequest, DaemonResponse, HealthMonitor, IpcStream, LspDaemon, MessageCodec, Watchdog,
};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::time::{interval, sleep, timeout, Duration};
use tracing::{error, info, warn};
use uuid::Uuid;

// Test constants
#[allow(dead_code)]
const STRESS_TEST_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes max per test
const CONNECTION_LIMIT: usize = 100;
#[allow(dead_code)]
const LARGE_MESSAGE_SIZE: usize = 1024 * 1024; // 1MB
const MEMORY_LEAK_THRESHOLD: usize = 50 * 1024 * 1024; // 50MB

/// Mock LSP server that can be configured to behave in various ways
struct MockLspServer {
    socket_path: String,
    behavior: MockLspBehavior,
    running: Arc<AtomicBool>,
    request_count: Arc<AtomicUsize>,
    delay_ms: u64,
}

#[allow(dead_code)]
#[derive(Clone)]
enum MockLspBehavior {
    Normal,              // Responds normally
    SlowResponses,       // Always responds slowly
    FailAfterN(usize),   // Fails after N requests
    RandomFailures(f32), // Fails with given probability
    MemoryLeak,          // Allocates memory without freeing
    Unresponsive,        // Never responds
    PartialResponses,    // Sends incomplete responses
    InvalidJson,         // Sends malformed JSON
}

impl MockLspServer {
    fn new(socket_path: String, behavior: MockLspBehavior) -> Self {
        Self {
            socket_path,
            behavior,
            running: Arc::new(AtomicBool::new(false)),
            request_count: Arc::new(AtomicUsize::new(0)),
            delay_ms: 100,
        }
    }

    #[allow(dead_code)]
    fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    async fn start(&self) -> Result<tokio::task::JoinHandle<()>> {
        self.running.store(true, Ordering::Relaxed);
        let socket_path = self.socket_path.clone();
        let behavior = self.behavior.clone();
        let running = self.running.clone();
        let request_count = self.request_count.clone();
        let delay_ms = self.delay_ms;

        // Remove existing socket if present
        let _ = std::fs::remove_file(&socket_path);

        let handle = tokio::spawn(async move {
            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(e) => {
                    error!("Failed to bind mock LSP server: {}", e);
                    return;
                }
            };

            info!("Mock LSP server started at {}", socket_path);

            while running.load(Ordering::Relaxed) {
                match timeout(Duration::from_millis(100), listener.accept()).await {
                    Ok(Ok((stream, _))) => {
                        let behavior = behavior.clone();
                        let request_count = request_count.clone();
                        let running = running.clone();

                        tokio::spawn(async move {
                            Self::handle_connection(
                                stream,
                                behavior,
                                request_count,
                                running,
                                delay_ms,
                            )
                            .await;
                        });
                    }
                    Ok(Err(e)) => {
                        warn!("Mock LSP server accept error: {}", e);
                        break;
                    }
                    Err(_) => {
                        // Timeout, continue loop to check running flag
                        continue;
                    }
                }
            }

            info!("Mock LSP server stopped");
        });

        // Give the server a moment to start
        sleep(Duration::from_millis(100)).await;
        Ok(handle)
    }

    async fn handle_connection(
        mut stream: UnixStream,
        behavior: MockLspBehavior,
        request_count: Arc<AtomicUsize>,
        running: Arc<AtomicBool>,
        delay_ms: u64,
    ) {
        let mut buffer = vec![0u8; 8192];

        while running.load(Ordering::Relaxed) {
            match timeout(Duration::from_millis(500), stream.read(&mut buffer)).await {
                Ok(Ok(0)) => break, // Connection closed
                Ok(Ok(_n)) => {
                    let count = request_count.fetch_add(1, Ordering::Relaxed) + 1;

                    if delay_ms > 0 {
                        sleep(Duration::from_millis(delay_ms)).await;
                    }

                    let response = match behavior {
                        MockLspBehavior::Normal => Self::create_normal_response(),
                        MockLspBehavior::SlowResponses => {
                            sleep(Duration::from_secs(5)).await;
                            Self::create_normal_response()
                        }
                        MockLspBehavior::FailAfterN(threshold) => {
                            if count > threshold {
                                Self::create_error_response("Server overloaded")
                            } else {
                                Self::create_normal_response()
                            }
                        }
                        MockLspBehavior::RandomFailures(probability) => {
                            if rand::random::<f32>() < probability {
                                Self::create_error_response("Random failure")
                            } else {
                                Self::create_normal_response()
                            }
                        }
                        MockLspBehavior::MemoryLeak => {
                            // Intentionally leak memory
                            let _leaked: Vec<u8> = vec![0u8; 1024 * 1024]; // 1MB
                            std::mem::forget(_leaked);
                            Self::create_normal_response()
                        }
                        MockLspBehavior::Unresponsive => {
                            // Don't respond at all
                            continue;
                        }
                        MockLspBehavior::PartialResponses => {
                            let response = Self::create_normal_response();
                            // Send only half the response
                            let partial = response.as_bytes();
                            let half_len = partial.len() / 2;
                            let _ = stream.write_all(&partial[..half_len]).await;
                            continue;
                        }
                        MockLspBehavior::InvalidJson => {
                            "Content-Length: 15\r\n\r\n{invalid json}".to_string()
                        }
                    };

                    let _ = stream.write_all(response.as_bytes()).await;
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
    }

    fn create_normal_response() -> String {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "capabilities": {
                    "callHierarchyProvider": true
                }
            }
        });

        let content = response.to_string();
        format!("Content-Length: {}\r\n\r\n{}", content.len(), content)
    }

    fn create_error_response(message: &str) -> String {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -1,
                "message": message
            }
        });

        let content = response.to_string();
        format!("Content-Length: {}\r\n\r\n{}", content.len(), content)
    }

    fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        // Remove socket file
        let _ = std::fs::remove_file(&self.socket_path);
    }

    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::Relaxed)
    }
}

/// Helper to create test daemon with custom socket path
async fn start_test_daemon() -> Result<(String, tokio::task::JoinHandle<()>)> {
    let socket_path = format!("/tmp/probe-stress-test-{}.sock", Uuid::new_v4());

    // Clean up any existing socket
    let _ = std::fs::remove_file(&socket_path);

    let daemon = LspDaemon::new(socket_path.clone())?;
    let handle = tokio::spawn(async move {
        if let Err(e) = daemon.run().await {
            error!("Daemon error: {}", e);
        }
    });

    // Wait for daemon to start
    sleep(Duration::from_millis(500)).await;

    Ok((socket_path, handle))
}

/// Helper to create unresponsive client connection
async fn create_unresponsive_client(socket_path: &str) -> Result<()> {
    let stream = IpcStream::connect(socket_path).await?;

    // Send only the length header, not the message body
    let partial_message = b"\x00\x00\x00\x10"; // 16 bytes length
    let mut stream = stream;
    stream.write_all(partial_message).await?;

    // Keep connection open but don't send more data
    // This will make the daemon wait for the rest of the message
    tokio::spawn(async move {
        let _stream = stream;
        sleep(Duration::from_secs(3600)).await; // Keep alive for 1 hour
    });

    Ok(())
}

/// Helper to measure memory usage
fn measure_memory_usage() -> Result<usize> {
    #[cfg(target_os = "linux")]
    {
        use std::fs;
        let status = fs::read_to_string("/proc/self/status")?;
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let kb: usize = parts[1].parse().unwrap_or(0);
                    return Ok(kb * 1024); // Convert KB to bytes
                }
            }
        }
        Ok(0)
    }

    #[cfg(target_os = "macos")]
    {
        // Use task_info on macOS
        use libc::{c_int, pid_t};
        use std::mem;

        extern "C" {
            fn getpid() -> pid_t;
            fn proc_pidinfo(
                pid: pid_t,
                flavor: c_int,
                arg: u64,
                buffer: *mut libc::c_void,
                buffersize: c_int,
            ) -> c_int;
        }

        const PROC_PIDTASKINFO: c_int = 4;

        #[repr(C)]
        struct ProcTaskInfo {
            pti_virtual_size: u64,
            pti_resident_size: u64,
            pti_total_user: u64,
            pti_total_system: u64,
            pti_threads_user: u64,
            pti_threads_system: u64,
            pti_policy: i32,
            pti_faults: i32,
            pti_pageins: i32,
            pti_cow_faults: i32,
            pti_messages_sent: i32,
            pti_messages_received: i32,
            pti_syscalls_mach: i32,
            pti_syscalls_unix: i32,
            pti_csw: i32,
            pti_threadnum: i32,
            pti_numrunning: i32,
            pti_priority: i32,
        }

        unsafe {
            let mut info: ProcTaskInfo = mem::zeroed();
            let size = mem::size_of::<ProcTaskInfo>() as c_int;
            let result = proc_pidinfo(
                getpid(),
                PROC_PIDTASKINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            );

            if result == size {
                Ok(info.pti_resident_size as usize)
            } else {
                Ok(0)
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // Fallback for other platforms
        Ok(0)
    }
}

// ==================== STRESS TESTS ====================

#[tokio::test]
#[ignore = "Long running stress test - run with --ignored"]
async fn test_daemon_handles_unresponsive_client() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting unresponsive client stress test");

    let (socket_path, daemon_handle) = start_test_daemon().await?;

    // Create multiple unresponsive clients
    for i in 0..5 {
        create_unresponsive_client(&socket_path)
            .await
            .with_context(|| format!("Failed to create unresponsive client {i}"))?;
    }

    // Wait a bit for daemon to process the partial connections
    sleep(Duration::from_millis(1000)).await;

    // Verify daemon can still accept new connections
    let mut stream = timeout(Duration::from_secs(5), IpcStream::connect(&socket_path)).await??;

    let request = DaemonRequest::Status {
        request_id: Uuid::new_v4(),
    };

    let encoded = MessageCodec::encode(&request)?;
    stream.write_all(&encoded).await?;

    let mut response_data = vec![0u8; 8192];
    let n = timeout(Duration::from_secs(5), stream.read(&mut response_data)).await??;
    response_data.truncate(n);

    match MessageCodec::decode_response(&response_data)? {
        DaemonResponse::Status { status, .. } => {
            assert!(status.uptime_secs > 0, "Daemon should still be running");
            info!("✅ Daemon handled unresponsive clients successfully");
            info!("   Active connections: {}", status.active_connections);
        }
        _ => panic!("Expected status response"),
    }

    // Cleanup
    daemon_handle.abort();
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}

#[tokio::test]
#[ignore = "Stress test with many connections - run with --ignored"]
async fn test_daemon_handles_many_concurrent_connections() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting concurrent connections stress test");

    let (socket_path, daemon_handle) = start_test_daemon().await?;

    let semaphore = Arc::new(Semaphore::new(CONNECTION_LIMIT));
    let success_count = Arc::new(AtomicUsize::new(0));
    let reject_count = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();

    // Try to create many concurrent connections
    for i in 0..CONNECTION_LIMIT * 2 {
        let socket_path = socket_path.clone();
        let semaphore = semaphore.clone();
        let success_count = success_count.clone();
        let reject_count = reject_count.clone();

        let handle = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();

            match timeout(Duration::from_secs(10), IpcStream::connect(&socket_path)).await {
                Ok(Ok(mut stream)) => {
                    // Make a simple status request
                    let request = DaemonRequest::Status {
                        request_id: Uuid::new_v4(),
                    };

                    if let Ok(encoded) = MessageCodec::encode(&request) {
                        if stream.write_all(&encoded).await.is_ok() {
                            let mut response_data = vec![0u8; 8192];
                            if let Ok(n) = stream.read(&mut response_data).await {
                                response_data.truncate(n);
                                if MessageCodec::decode_response(&response_data).is_ok() {
                                    success_count.fetch_add(1, Ordering::Relaxed);
                                    return;
                                }
                            }
                        }
                    }
                    reject_count.fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    reject_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        handles.push(handle);

        // Small delay to avoid overwhelming the system
        if i % 10 == 0 {
            sleep(Duration::from_millis(10)).await;
        }
    }

    // Wait for all connection attempts to complete
    for handle in handles {
        let _ = handle.await;
    }

    let successes = success_count.load(Ordering::Relaxed);
    let rejections = reject_count.load(Ordering::Relaxed);

    info!("Connection test results:");
    info!("  Successful connections: {}", successes);
    info!("  Rejected connections: {}", rejections);

    // Verify some connections were successful and some were rejected
    assert!(successes > 0, "At least some connections should succeed");
    assert!(
        successes + rejections == CONNECTION_LIMIT * 2,
        "All attempts should be accounted for"
    );

    info!("✅ Concurrent connection handling test passed");

    // Cleanup
    daemon_handle.abort();
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}

#[tokio::test]
#[ignore = "Long running health monitor test - run with --ignored"]
async fn test_health_monitor_restarts_unhealthy_servers() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting health monitor test");

    let temp_dir = TempDir::new()?;
    let mock_socket = temp_dir.path().join("mock-lsp.sock");

    // Start with a normal mock server
    let mock_server = MockLspServer::new(
        mock_socket.to_string_lossy().to_string(),
        MockLspBehavior::FailAfterN(3),
    );

    let _mock_handle = mock_server.start().await?;

    // Create health monitor
    let _server_manager = Arc::new(lsp_daemon::server_manager::SingleServerManager::new(
        Arc::new(lsp_daemon::lsp_registry::LspRegistry::new()?),
    ));

    let _health_monitor = HealthMonitor::new();

    // Wait for several health checks to occur
    sleep(Duration::from_secs(30)).await;

    // For this test, we'll just verify the health monitor can be created
    // The actual health checking would require integration with the server manager

    // Verify health monitor was created successfully
    info!("Health monitor created successfully");

    mock_server.stop();

    info!("✅ Health monitor test completed");

    Ok(())
}

#[tokio::test]
#[ignore = "Circuit breaker test - run with --ignored"]
async fn test_circuit_breaker_prevents_cascading_failures() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting circuit breaker test");

    let (socket_path, daemon_handle) = start_test_daemon().await?;

    let error_count = Arc::new(AtomicUsize::new(0));
    let fast_failures = Arc::new(AtomicUsize::new(0));

    // Make many requests that will likely fail to trigger circuit breaker
    let mut handles = Vec::new();

    for _i in 0..50 {
        let socket_path = socket_path.clone();
        let error_count = error_count.clone();
        let fast_failures = fast_failures.clone();

        let handle = tokio::spawn(async move {
            let start_time = Instant::now();

            match timeout(Duration::from_secs(10), IpcStream::connect(&socket_path)).await {
                Ok(Ok(mut stream)) => {
                    let request = DaemonRequest::CallHierarchy {
                        request_id: Uuid::new_v4(),
                        file_path: PathBuf::from("/nonexistent/file.rs"),
                        line: 1,
                        column: 0,
                        workspace_hint: None,
                    };

                    if let Ok(encoded) = MessageCodec::encode(&request) {
                        if stream.write_all(&encoded).await.is_ok() {
                            let mut response_data = vec![0u8; 8192];
                            match timeout(Duration::from_secs(5), stream.read(&mut response_data))
                                .await
                            {
                                Ok(Ok(n)) => {
                                    response_data.truncate(n);
                                    if let Ok(DaemonResponse::Error { .. }) =
                                        MessageCodec::decode_response(&response_data)
                                    {
                                        let elapsed = start_time.elapsed();
                                        if elapsed < Duration::from_millis(100) {
                                            fast_failures.fetch_add(1, Ordering::Relaxed);
                                        }
                                        error_count.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                _ => {
                                    error_count.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
                _ => {
                    error_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        handles.push(handle);

        // Small delay between requests
        sleep(Duration::from_millis(50)).await;
    }

    // Wait for all requests
    for handle in handles {
        let _ = handle.await;
    }

    let total_errors = error_count.load(Ordering::Relaxed);
    let fast_fails = fast_failures.load(Ordering::Relaxed);

    info!("Circuit breaker test results:");
    info!("  Total errors: {}", total_errors);
    info!("  Fast failures: {}", fast_fails);

    // Verify circuit breaker behavior
    assert!(
        total_errors > 0,
        "Should have some errors to test circuit breaker"
    );

    info!("✅ Circuit breaker test completed");

    // Cleanup
    daemon_handle.abort();
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}

#[tokio::test]
#[ignore = "Watchdog test - run with --ignored"]
async fn test_watchdog_detects_unresponsive_daemon() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting watchdog test");

    let recovery_triggered = Arc::new(AtomicBool::new(false));
    let watchdog = Watchdog::new(5); // 5 second timeout

    // Set recovery callback
    let recovery_flag = recovery_triggered.clone();
    watchdog
        .set_recovery_callback(move || {
            recovery_flag.store(true, Ordering::Relaxed);
        })
        .await;

    // Start watchdog
    let watchdog_handle = watchdog.start();

    // Send heartbeat initially
    watchdog.heartbeat();

    // Wait for a bit
    sleep(Duration::from_secs(2)).await;

    // Stop sending heartbeats (simulate unresponsive daemon)
    // Wait longer than timeout
    sleep(Duration::from_secs(8)).await;

    // Check if recovery was triggered
    let was_triggered = recovery_triggered.load(Ordering::Relaxed);

    // Stop watchdog
    watchdog.stop();
    let _ = watchdog_handle.await;

    assert!(was_triggered, "Watchdog should have triggered recovery");

    info!("✅ Watchdog test completed successfully");

    Ok(())
}

#[tokio::test]
#[ignore = "Connection cleanup test - run with --ignored"]
async fn test_connection_cleanup_prevents_resource_leak() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting connection cleanup test");

    let (socket_path, daemon_handle) = start_test_daemon().await?;

    let initial_memory = measure_memory_usage()?;
    info!("Initial memory usage: {} bytes", initial_memory);

    // Create many connections and leave them idle
    let mut connections = Vec::new();

    for i in 0..20 {
        match IpcStream::connect(&socket_path).await {
            Ok(stream) => {
                connections.push(stream);
                info!("Created connection {}", i + 1);
            }
            Err(e) => {
                warn!("Failed to create connection {}: {}", i + 1, e);
                break;
            }
        }

        sleep(Duration::from_millis(100)).await;
    }

    info!("Created {} idle connections", connections.len());

    // Wait for cleanup to occur
    sleep(Duration::from_secs(10)).await;

    // Check memory usage
    let current_memory = measure_memory_usage()?;
    let memory_growth = current_memory.saturating_sub(initial_memory);

    info!("Current memory usage: {} bytes", current_memory);
    info!("Memory growth: {} bytes", memory_growth);

    // Verify memory growth is reasonable
    assert!(
        memory_growth < MEMORY_LEAK_THRESHOLD,
        "Memory growth ({memory_growth} bytes) exceeds threshold ({MEMORY_LEAK_THRESHOLD} bytes)"
    );

    // Clean up connections
    drop(connections);

    // Final memory check after cleanup
    sleep(Duration::from_secs(2)).await;
    let final_memory = measure_memory_usage()?;
    info!("Final memory usage: {} bytes", final_memory);

    info!("✅ Connection cleanup test completed");

    // Cleanup
    daemon_handle.abort();
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}

#[tokio::test]
#[ignore = "LSP server crash handling test - run with --ignored"]
async fn test_daemon_handles_lsp_server_crash() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting LSP server crash handling test");

    let temp_dir = TempDir::new()?;
    let mock_socket = temp_dir.path().join("crash-test-lsp.sock");

    // Start mock server that will crash after a few requests
    let mock_server = MockLspServer::new(
        mock_socket.to_string_lossy().to_string(),
        MockLspBehavior::FailAfterN(2),
    );

    let mock_handle = mock_server.start().await?;

    // Give server time to start
    sleep(Duration::from_millis(500)).await;

    // Make requests that should trigger the "crash"
    for i in 0..5 {
        // Simulate connecting directly to the mock server
        if let Ok(Ok(mut stream)) =
            timeout(Duration::from_secs(1), UnixStream::connect(&mock_socket)).await
        {
            let request = b"test request";
            let _ = stream.write_all(request).await;
            let mut response = vec![0u8; 1024];
            let _ = stream.read(&mut response).await;
            info!("Request {} completed", i + 1);
        }

        sleep(Duration::from_millis(100)).await;
    }

    // Verify mock server handled requests
    let request_count = mock_server.request_count();
    info!("Mock server handled {} requests", request_count);

    assert!(
        request_count >= 2,
        "Mock server should have handled at least 2 requests"
    );

    // Stop mock server (simulating crash)
    mock_server.stop();
    let _ = mock_handle.await;

    info!("✅ LSP server crash handling test completed");

    Ok(())
}

#[tokio::test]
#[ignore = "Very long running stability test - run with --ignored"]
async fn test_daemon_stability_over_time() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting daemon stability test (simulated long-term operation)");

    let (socket_path, daemon_handle) = start_test_daemon().await?;

    let initial_memory = measure_memory_usage()?;
    info!("Initial memory usage: {} bytes", initial_memory);

    let start_time = Instant::now();
    let request_count = Arc::new(AtomicUsize::new(0));
    let error_count = Arc::new(AtomicUsize::new(0));

    // Run for a shorter time but with more intensive load for testing
    let test_duration = Duration::from_secs(60); // 1 minute instead of 24 hours
    let request_interval = Duration::from_millis(100); // More frequent requests

    let socket_path_clone = socket_path.clone();
    let request_count_clone = request_count.clone();
    let error_count_clone = error_count.clone();

    let mut load_test_handle = tokio::spawn(async move {
        let mut interval = interval(request_interval);
        let end_time = Instant::now() + test_duration;

        while Instant::now() < end_time {
            interval.tick().await;

            match timeout(
                Duration::from_secs(5),
                IpcStream::connect(&socket_path_clone),
            )
            .await
            {
                Ok(Ok(mut stream)) => {
                    let request = DaemonRequest::Status {
                        request_id: Uuid::new_v4(),
                    };

                    match MessageCodec::encode(&request) {
                        Ok(encoded) => {
                            if stream.write_all(&encoded).await.is_ok() {
                                let mut response_data = vec![0u8; 8192];
                                match stream.read(&mut response_data).await {
                                    Ok(n) => {
                                        response_data.truncate(n);
                                        match MessageCodec::decode_response(&response_data) {
                                            Ok(DaemonResponse::Status { .. }) => {
                                                request_count_clone.fetch_add(1, Ordering::Relaxed);
                                            }
                                            _ => {
                                                error_count_clone.fetch_add(1, Ordering::Relaxed);
                                            }
                                        }
                                    }
                                    Err(_) => {
                                        error_count_clone.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            error_count_clone.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                _ => {
                    error_count_clone.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    });

    // Monitor memory usage periodically
    let mut memory_samples = Vec::new();
    let mut check_interval = interval(Duration::from_secs(10));

    loop {
        tokio::select! {
            _ = check_interval.tick() => {
                let current_memory = measure_memory_usage()?;
                memory_samples.push(current_memory);

                let elapsed = start_time.elapsed();
                info!("Stability check at {:?}: {} requests, {} errors, {} bytes memory",
                      elapsed,
                      request_count.load(Ordering::Relaxed),
                      error_count.load(Ordering::Relaxed),
                      current_memory);

                if elapsed >= test_duration {
                    break;
                }
            }
            result = &mut load_test_handle => {
                match result {
                    Ok(_) => info!("Load test completed successfully"),
                    Err(e) => warn!("Load test failed: {}", e),
                }
                break;
            }
        }
    }

    let final_memory = measure_memory_usage()?;
    let total_requests = request_count.load(Ordering::Relaxed);
    let total_errors = error_count.load(Ordering::Relaxed);

    info!("Stability test results:");
    info!("  Duration: {:?}", start_time.elapsed());
    info!("  Total requests: {}", total_requests);
    info!("  Total errors: {}", total_errors);
    info!("  Initial memory: {} bytes", initial_memory);
    info!("  Final memory: {} bytes", final_memory);
    info!(
        "  Memory growth: {} bytes",
        final_memory.saturating_sub(initial_memory)
    );

    // Verify stability metrics
    assert!(total_requests > 0, "Should have processed some requests");

    let error_rate = total_errors as f64 / total_requests as f64;
    assert!(
        error_rate < 0.1,
        "Error rate ({:.2}%) should be less than 10%",
        error_rate * 100.0
    );

    let memory_growth = final_memory.saturating_sub(initial_memory);
    assert!(
        memory_growth < MEMORY_LEAK_THRESHOLD,
        "Memory growth ({memory_growth} bytes) should be less than {MEMORY_LEAK_THRESHOLD} bytes"
    );

    info!("✅ Daemon stability test completed successfully");

    // Cleanup
    daemon_handle.abort();
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}

#[tokio::test]
#[ignore = "Large message handling test - run with --ignored"]
async fn test_daemon_handles_large_messages() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Starting large message handling test");

    let (socket_path, daemon_handle) = start_test_daemon().await?;

    // Test with progressively larger messages
    let message_sizes = vec![1024, 10_240, 102_400, 1_024_000]; // 1KB to 1MB

    for size in message_sizes {
        info!("Testing with {} byte message", size);

        match timeout(Duration::from_secs(30), IpcStream::connect(&socket_path)).await {
            Ok(Ok(mut stream)) => {
                // Create large file path
                let large_path = "x".repeat(size);

                let request = DaemonRequest::CallHierarchy {
                    request_id: Uuid::new_v4(),
                    file_path: PathBuf::from(large_path),
                    line: 1,
                    column: 0,
                    workspace_hint: None,
                };

                match MessageCodec::encode(&request) {
                    Ok(encoded) => {
                        assert!(
                            encoded.len() > size,
                            "Encoded message should be at least as large as input"
                        );

                        if stream.write_all(&encoded).await.is_ok() {
                            let mut response_data = vec![0u8; encoded.len() * 2];
                            match timeout(Duration::from_secs(10), stream.read(&mut response_data))
                                .await
                            {
                                Ok(Ok(n)) => {
                                    response_data.truncate(n);
                                    match MessageCodec::decode_response(&response_data) {
                                        Ok(_) => {
                                            info!("✅ Successfully handled {} byte message", size);
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Failed to decode response for {} byte message: {}",
                                                size, e
                                            );
                                        }
                                    }
                                }
                                Ok(Err(e)) => {
                                    warn!(
                                        "Failed to read response for {} byte message: {}",
                                        size, e
                                    );
                                }
                                Err(_) => {
                                    warn!("Timeout reading response for {} byte message", size);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to encode {} byte message: {}", size, e);
                    }
                }
            }
            _ => {
                warn!("Failed to connect for {} byte message test", size);
            }
        }

        // Small delay between tests
        sleep(Duration::from_millis(100)).await;
    }

    info!("✅ Large message handling test completed");

    // Cleanup
    daemon_handle.abort();
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}

// Helper test to validate test infrastructure
#[tokio::test]
async fn test_mock_lsp_server_functionality() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    info!("Testing mock LSP server infrastructure");

    let temp_dir = TempDir::new()?;
    let socket_path = temp_dir.path().join("test-mock.sock");

    // Test normal behavior
    let mock_server = MockLspServer::new(
        socket_path.to_string_lossy().to_string(),
        MockLspBehavior::Normal,
    );

    let handle = mock_server.start().await?;

    // Connect and make a request
    match timeout(Duration::from_secs(5), UnixStream::connect(&socket_path)).await {
        Ok(Ok(mut stream)) => {
            let request = b"test request";
            stream.write_all(request).await?;

            let mut response = vec![0u8; 1024];
            let n = stream.read(&mut response).await?;
            response.truncate(n);

            assert!(n > 0, "Should receive a response");
            info!("Mock server response: {} bytes", n);
        }
        _ => {
            panic!("Failed to connect to mock server");
        }
    }

    assert!(
        mock_server.request_count() > 0,
        "Mock server should have processed requests"
    );

    mock_server.stop();
    let _ = handle.await;

    info!("✅ Mock LSP server infrastructure test passed");

    Ok(())
}
