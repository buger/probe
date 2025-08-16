use anyhow::{anyhow, Context, Result};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process;
use std::time::Duration;
use tracing::{debug, info, warn};

/// PID file lock to ensure only one daemon instance runs at a time
pub struct PidLock {
    path: PathBuf,
    file: Option<File>,
    locked: bool,
}

impl PidLock {
    /// Create a new PID lock at the specified path
    pub fn new(socket_path: &str) -> Self {
        // Create PID file path based on socket path
        let pid_path = format!("{socket_path}.pid");
        Self {
            path: PathBuf::from(pid_path),
            file: None,
            locked: false,
        }
    }

    /// Try to acquire the lock, returning Ok if successful
    pub fn try_lock(&mut self) -> Result<()> {
        // Use a lock file to coordinate multiple processes trying to acquire the PID lock
        let lock_path = format!("{}.lock", self.path.display());
        let lock_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .context("Failed to open coordination lock file")?;

        // Use exclusive file locking to ensure only one process can proceed
        lock_file.try_lock_exclusive().map_err(|_| {
            anyhow!("Another process is currently trying to acquire the daemon lock")
        })?;

        // Now we have exclusive access to check and create the PID file
        let result = self.try_lock_internal();

        // Always release the coordination lock
        let _ = FileExt::unlock(&lock_file);
        // Clean up the coordination lock file
        let _ = fs::remove_file(&lock_path);

        result
    }

    /// Internal lock acquisition with atomic operations
    fn try_lock_internal(&mut self) -> Result<()> {
        let pid = process::id();

        // First, check if a PID file exists and if that process is still running
        if self.path.exists() {
            match File::open(&self.path) {
                Ok(mut file) => {
                    let mut contents = String::new();
                    file.read_to_string(&mut contents)
                        .context("Failed to read PID file")?;

                    let existing_pid: u32 = contents
                        .trim()
                        .parse()
                        .context("Invalid PID in lock file")?;

                    if is_process_running(existing_pid) {
                        // Try to lock the existing file to verify it's really in use
                        if file.try_lock_exclusive().is_err() {
                            return Err(anyhow!(
                                "Another daemon instance is already running (PID: {})",
                                existing_pid
                            ));
                        }
                        // If we can lock it, the process might be dead but file wasn't cleaned up
                        let _ = FileExt::unlock(&file);
                    }

                    warn!(
                        "Found stale PID file for non-running process {}, removing",
                        existing_pid
                    );
                    drop(file); // Close the file before removing
                    fs::remove_file(&self.path).context("Failed to remove stale PID file")?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // File was removed between exists() check and open(), continue
                }
                Err(e) => return Err(e).context("Failed to open existing PID file"),
            }
        }

        // Try to create the PID file atomically using create_new (O_CREAT | O_EXCL)
        let file = match OpenOptions::new()
            .write(true)
            .create_new(true) // This is atomic - fails if file exists
            .open(&self.path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Another process created the file between our check and create
                // Try one more time with a small delay
                std::thread::sleep(Duration::from_millis(50));
                return self.try_lock_internal();
            }
            Err(e) => return Err(e).context("Failed to create PID file"),
        };

        // Lock the file exclusively to prevent other processes from reading/writing
        file.try_lock_exclusive()
            .map_err(|_| anyhow!("Failed to acquire exclusive lock on PID file"))?;

        // Write our PID to the file
        let mut file = file; // Make mutable for writing
        write!(file, "{pid}").context("Failed to write PID to lock file")?;
        file.flush().context("Failed to flush PID file")?;

        self.file = Some(file);
        self.locked = true;
        info!("Acquired PID lock at {:?} (PID: {})", self.path, pid);
        Ok(())
    }

    /// Release the lock by removing the PID file
    pub fn unlock(&mut self) -> Result<()> {
        if !self.locked {
            return Ok(());
        }

        // Unlock and close the file
        if let Some(file) = self.file.take() {
            let _ = FileExt::unlock(&file);
            drop(file);
        }

        if self.path.exists() {
            // Verify it's our PID before removing
            let mut file = File::open(&self.path)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;

            let pid: u32 = contents.trim().parse().unwrap_or(0);

            if pid == process::id() {
                fs::remove_file(&self.path).context("Failed to remove PID file")?;
                debug!("Released PID lock at {:?}", self.path);
            } else {
                warn!("PID file contains different PID ({}), not removing", pid);
            }
        }

        self.locked = false;
        Ok(())
    }

    /// Check if the lock is currently held by this instance
    pub fn is_locked(&self) -> bool {
        self.locked
    }
}

impl Drop for PidLock {
    fn drop(&mut self) {
        if self.locked {
            if let Err(e) = self.unlock() {
                warn!("Failed to unlock PID file on drop: {}", e);
            }
        }
    }
}

/// Check if a process with the given PID is running
#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    // On Unix, we can check if a process exists by sending signal 0
    // This doesn't actually send a signal, just checks if we could
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            false
        } else {
            CloseHandle(handle);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn test_pid_lock_exclusive() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("test.sock").to_str().unwrap().to_string();

        let mut lock1 = PidLock::new(&socket_path);
        assert!(lock1.try_lock().is_ok(), "First lock should succeed");

        let mut lock2 = PidLock::new(&socket_path);
        assert!(lock2.try_lock().is_err(), "Second lock should fail");

        lock1.unlock().unwrap();
        assert!(lock2.try_lock().is_ok(), "Lock should succeed after unlock");
    }

    #[test]
    fn test_stale_pid_cleanup() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("test.sock").to_str().unwrap().to_string();
        let pid_path = format!("{socket_path}.pid");

        // Write a non-existent PID to the file
        std::fs::write(&pid_path, "99999999").unwrap();

        let mut lock = PidLock::new(&socket_path);
        assert!(
            lock.try_lock().is_ok(),
            "Should acquire lock after cleaning stale PID"
        );
        assert_eq!(
            std::fs::read_to_string(&pid_path).unwrap().trim(),
            process::id().to_string(),
            "PID file should contain current process ID"
        );
    }

    #[test]
    fn test_concurrent_lock_attempts() {
        let dir = tempdir().unwrap();
        let socket_path = Arc::new(dir.path().join("test.sock").to_str().unwrap().to_string());
        let barrier = Arc::new(Barrier::new(5));
        let success_count = Arc::new(std::sync::Mutex::new(0));

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let socket_path = Arc::clone(&socket_path);
                let barrier = Arc::clone(&barrier);
                let success_count = Arc::clone(&success_count);

                thread::spawn(move || {
                    barrier.wait(); // Ensure all threads start at the same time

                    let mut lock = PidLock::new(&socket_path);
                    if lock.try_lock().is_ok() {
                        *success_count.lock().unwrap() += 1;
                        // Hold the lock briefly
                        thread::sleep(Duration::from_millis(10));
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(
            *success_count.lock().unwrap(),
            1,
            "Exactly one thread should acquire the lock"
        );
    }
}
