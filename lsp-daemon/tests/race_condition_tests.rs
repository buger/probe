use anyhow::Result;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

/// Test that only one daemon can start even with multiple concurrent attempts
#[test]
fn test_multiple_daemon_startup_attempts() -> Result<()> {
    let dir = tempdir()?;
    let socket_path = dir.path().join("test.sock").to_str().unwrap().to_string();

    // Create a barrier to synchronize thread starts
    let barrier = Arc::new(Barrier::new(5));
    let socket_path = Arc::new(socket_path);
    let success_count = Arc::new(std::sync::Mutex::new(0));
    let error_messages = Arc::new(std::sync::Mutex::new(Vec::new()));

    // Spawn 5 threads that all try to start a daemon at the same time
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let barrier = Arc::clone(&barrier);
            let socket_path = Arc::clone(&socket_path);
            let success_count = Arc::clone(&success_count);
            let error_messages = Arc::clone(&error_messages);

            thread::spawn(move || {
                // Wait for all threads to be ready
                barrier.wait();

                // Try to acquire PID lock
                let mut pid_lock = lsp_daemon::pid_lock::PidLock::new(&socket_path);
                match pid_lock.try_lock() {
                    Ok(()) => {
                        *success_count.lock().unwrap() += 1;
                        println!("Thread {i} acquired lock");
                        // Hold the lock for a bit to simulate daemon running
                        thread::sleep(Duration::from_millis(100));
                        let _ = pid_lock.unlock();
                    }
                    Err(e) => {
                        error_messages
                            .lock()
                            .unwrap()
                            .push(format!("Thread {i} failed: {e}"));
                    }
                }
            })
        })
        .collect();

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify only one thread succeeded
    let successes = *success_count.lock().unwrap();
    let errors = error_messages.lock().unwrap();

    println!("Successes: {successes}");
    println!("Errors: {errors:?}");

    assert_eq!(successes, 1, "Exactly one daemon should start successfully");
    assert_eq!(errors.len(), 4, "Four attempts should fail");

    Ok(())
}

/// Test that socket binding is properly coordinated
#[tokio::test]
async fn test_socket_binding_race_condition() -> Result<()> {
    let dir = tempdir()?;
    let socket_path = dir.path().join("test.sock").to_str().unwrap().to_string();

    // Create multiple tasks that try to bind to the same socket
    let socket_path = Arc::new(socket_path);
    let success_count = Arc::new(std::sync::Mutex::new(0));

    let mut handles = Vec::new();

    for i in 0..5 {
        let socket_path = Arc::clone(&socket_path);
        let success_count = Arc::clone(&success_count);

        let handle = tokio::spawn(async move {
            // Small random delay to increase chance of race
            tokio::time::sleep(Duration::from_millis(i * 10)).await;

            match lsp_daemon::ipc::IpcListener::bind(&socket_path).await {
                Ok(_listener) => {
                    *success_count.lock().unwrap() += 1;
                    println!("Task {i} bound to socket");
                    // Keep the listener alive
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => {
                    println!("Task {i} failed to bind: {e}");
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all tasks
    for handle in handles {
        let _ = handle.await;
    }

    let successes = *success_count.lock().unwrap();
    assert_eq!(
        successes, 1,
        "Only one task should successfully bind to socket"
    );

    Ok(())
}

/// Test PID lock cleanup after process crash
#[test]
fn test_stale_pid_lock_cleanup() -> Result<()> {
    let dir = tempdir()?;
    let socket_path = dir.path().join("test.sock").to_str().unwrap().to_string();
    let pid_file = format!("{socket_path}.pid");

    // Write a non-existent PID to simulate a crashed process
    std::fs::write(&pid_file, "99999999")?;

    // Try to acquire lock - should succeed after cleaning up stale PID
    let mut pid_lock = lsp_daemon::pid_lock::PidLock::new(&socket_path);
    assert!(
        pid_lock.try_lock().is_ok(),
        "Should acquire lock after cleaning stale PID"
    );

    // Verify the PID file now contains our PID
    let contents = std::fs::read_to_string(&pid_file)?;
    assert_eq!(
        contents.trim(),
        std::process::id().to_string(),
        "PID file should contain current process ID"
    );

    Ok(())
}

/// Test that client startup coordination prevents multiple daemon spawns
#[test]
fn test_client_startup_coordination() -> Result<()> {
    // This test would require the actual probe binary to be built
    // and would spawn multiple client processes
    // For now, we'll test the lock mechanism directly

    let dir = tempdir()?;
    let lock_path = dir.path().join("client-start.lock");

    // Simulate multiple clients trying to start daemon
    let barrier = Arc::new(Barrier::new(3));
    let lock_path = Arc::new(lock_path);
    let started_count = Arc::new(std::sync::Mutex::new(0));

    let handles: Vec<_> = (0..3)
        .map(|i| {
            let barrier = Arc::clone(&barrier);
            let lock_path = Arc::clone(&lock_path);
            let started_count = Arc::clone(&started_count);

            thread::spawn(move || {
                barrier.wait();

                // Try to create lock file atomically
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(lock_path.as_ref())
                {
                    Ok(_) => {
                        *started_count.lock().unwrap() += 1;
                        println!("Client {i} acquired startup lock");
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => {
                        println!("Client {i} failed to acquire startup lock");
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let started = *started_count.lock().unwrap();
    assert_eq!(started, 1, "Only one client should acquire startup lock");

    Ok(())
}

/// Stress test with many concurrent daemon start attempts
#[test]
fn test_stress_concurrent_daemon_starts() -> Result<()> {
    let dir = tempdir()?;
    let socket_path = dir.path().join("test.sock").to_str().unwrap().to_string();

    let barrier = Arc::new(Barrier::new(20));
    let socket_path = Arc::new(socket_path);
    let success_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let handles: Vec<_> = (0..20)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let socket_path = Arc::clone(&socket_path);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                let mut pid_lock = lsp_daemon::pid_lock::PidLock::new(&socket_path);
                if pid_lock.try_lock().is_ok() {
                    success_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(10));
                    let _ = pid_lock.unlock();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    let successes = success_count.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        successes, 1,
        "Exactly one daemon should start even under stress"
    );

    Ok(())
}
