#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;
use std::process::Child;
#[cfg(unix)]
use tracing::debug;
#[cfg(not(unix))]
use tracing::warn;

/// Helper for managing process groups to ensure child processes are cleaned up
#[derive(Default)]
pub struct ProcessGroup {
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl ProcessGroup {
    /// Create a new process group manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Set up the current process as a process group leader
    #[cfg(unix)]
    pub fn become_leader(&mut self) -> Result<()> {
        unsafe {
            let pid = libc::getpid();
            if libc::setpgid(0, pid) == 0 {
                self.pgid = Some(pid);
                debug!("Set process group ID to {}", pid);
                Ok(())
            } else {
                Err(anyhow::anyhow!("Failed to set process group ID"))
            }
        }
    }

    #[cfg(not(unix))]
    pub fn become_leader(&mut self) -> Result<()> {
        // Process groups are Unix-specific
        Ok(())
    }

    /// Add a child process to our process group
    #[cfg(unix)]
    pub fn add_child(&self, child: &Child) -> Result<()> {
        if let Some(pgid) = self.pgid {
            let child_pid = child.id();
            unsafe {
                if libc::setpgid(child_pid as i32, pgid) != 0 {
                    // This can fail if the child has already exec'd, which is okay
                    debug!(
                        "Could not add child {} to process group (may have already exec'd)",
                        child_pid
                    );
                } else {
                    debug!("Added child {} to process group {}", child_pid, pgid);
                }
            }
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn add_child(&self, _child: &Child) -> Result<()> {
        Ok(())
    }

    /// Kill all processes in the process group
    #[cfg(unix)]
    pub fn kill_all(&self) {
        if let Some(pgid) = self.pgid {
            unsafe {
                // Send SIGTERM to all processes in the group
                if libc::killpg(pgid, libc::SIGTERM) != 0 {
                    debug!("Failed to send SIGTERM to process group {}", pgid);
                } else {
                    debug!("Sent SIGTERM to process group {}", pgid);

                    // Give processes time to shutdown gracefully
                    std::thread::sleep(std::time::Duration::from_millis(500));

                    // Force kill any remaining processes
                    if libc::killpg(pgid, libc::SIGKILL) != 0 {
                        debug!("Failed to send SIGKILL to process group {}", pgid);
                    } else {
                        debug!("Sent SIGKILL to process group {}", pgid);
                    }
                }
            }
        }
    }

    #[cfg(not(unix))]
    pub fn kill_all(&self) {
        // Process groups are Unix-specific
        warn!("Process group management not available on this platform");
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Don't kill on drop - let the daemon handle this explicitly
    }
}

/// Kill all child processes of a given PID (recursively)
#[cfg(unix)]
pub fn kill_process_tree(pid: u32) -> Result<()> {
    use std::process::Command;

    // Get all child PIDs
    let output = Command::new("pgrep")
        .arg("-P")
        .arg(pid.to_string())
        .output()
        .context("Failed to find child processes")?;

    if output.status.success() {
        let child_pids = String::from_utf8_lossy(&output.stdout);
        for child_pid_str in child_pids.lines() {
            if let Ok(child_pid) = child_pid_str.trim().parse::<u32>() {
                // Recursively kill children of this child
                let _ = kill_process_tree(child_pid);

                // Kill this child
                unsafe {
                    libc::kill(child_pid as i32, libc::SIGTERM);
                }
                debug!("Killed child process {}", child_pid);
            }
        }
    }

    // Finally kill the parent
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }

    Ok(())
}

#[cfg(not(unix))]
pub fn kill_process_tree(_pid: u32) -> Result<()> {
    warn!("Process tree killing not implemented for this platform");
    Ok(())
}

/// Find all processes matching a name pattern (like rust-analyzer)
#[cfg(unix)]
pub fn find_processes_by_name(name_pattern: &str) -> Vec<u32> {
    use std::process::Command;

    let mut pids = Vec::new();

    // Use pgrep to find processes
    if let Ok(output) = Command::new("pgrep").arg("-f").arg(name_pattern).output() {
        if output.status.success() {
            let pid_str = String::from_utf8_lossy(&output.stdout);
            for line in pid_str.lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    pids.push(pid);
                }
            }
        }
    }

    pids
}

#[cfg(not(unix))]
pub fn find_processes_by_name(_name_pattern: &str) -> Vec<u32> {
    Vec::new()
}
