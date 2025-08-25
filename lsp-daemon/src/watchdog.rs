use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

/// Type alias for recovery callback to avoid complex type warning
type RecoveryCallback = Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>;

/// Watchdog to monitor daemon health and trigger recovery if needed
#[derive(Clone)]
pub struct Watchdog {
    /// Last heartbeat timestamp from main accept loop
    last_heartbeat: Arc<AtomicU64>,
    /// Whether the watchdog is running
    running: Arc<AtomicBool>,
    /// Timeout before considering the daemon unresponsive
    timeout: Duration,
    /// Callback to trigger when daemon is unresponsive
    recovery_callback: RecoveryCallback,
}

impl Watchdog {
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            last_heartbeat: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            timeout: Duration::from_secs(timeout_secs),
            recovery_callback: Arc::new(Mutex::new(None)),
        }
    }

    /// Update the heartbeat timestamp (called from main accept loop)
    pub fn heartbeat(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.last_heartbeat.store(now, Ordering::Relaxed);
    }

    /// Start the watchdog monitoring task
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        self.running.store(true, Ordering::Relaxed);
        let watchdog = self.clone();

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(10)); // Check every 10 seconds
            info!(
                "Watchdog started with {:.0}s timeout",
                watchdog.timeout.as_secs_f64()
            );

            while watchdog.running.load(Ordering::Relaxed) {
                interval.tick().await;

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let last_heartbeat = watchdog.last_heartbeat.load(Ordering::Relaxed);

                if last_heartbeat > 0 && now - last_heartbeat > watchdog.timeout.as_secs() {
                    error!(
                        "Watchdog: Main accept loop unresponsive for {} seconds (timeout: {}s)",
                        now - last_heartbeat,
                        watchdog.timeout.as_secs()
                    );

                    // Trigger recovery
                    if let Some(ref callback) = *watchdog.recovery_callback.lock().await {
                        warn!("Watchdog: Triggering recovery mechanism");
                        callback();
                    } else {
                        warn!("Watchdog: No recovery callback set, daemon may be unresponsive");
                    }
                }

                // Debug log every minute to show watchdog is alive
                if now % 60 == 0 {
                    debug!(
                        "Watchdog: Heartbeat age: {}s (timeout: {}s)",
                        if last_heartbeat > 0 {
                            now - last_heartbeat
                        } else {
                            0
                        },
                        watchdog.timeout.as_secs()
                    );
                }
            }

            info!("Watchdog monitoring stopped");
        })
    }

    /// Stop the watchdog
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        info!("Watchdog stop requested");
    }

    /// Set recovery callback
    pub async fn set_recovery_callback<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        *self.recovery_callback.lock().await = Some(Box::new(callback));
        info!("Watchdog recovery callback registered");
    }

    /// Get the current heartbeat age in seconds
    pub fn get_heartbeat_age(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let last_heartbeat = self.last_heartbeat.load(Ordering::Relaxed);

        if last_heartbeat > 0 {
            now - last_heartbeat
        } else {
            0
        }
    }

    /// Check if the watchdog considers the daemon healthy
    pub fn is_healthy(&self) -> bool {
        let age = self.get_heartbeat_age();
        age == 0 || age <= self.timeout.as_secs()
    }
}

/// Monitor LSP server process resource usage  
#[derive(Debug)]
pub struct ProcessMonitor {
    /// Maximum CPU percentage allowed (e.g., 80.0 for 80%)
    max_cpu_percent: f32,
    /// Maximum memory in MB
    max_memory_mb: u64,
    /// Timeout for getting process stats
    stats_timeout: Duration,
}

impl Default for ProcessMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessMonitor {
    pub fn new() -> Self {
        Self {
            max_cpu_percent: 80.0,
            max_memory_mb: 1024, // 1GB default
            stats_timeout: Duration::from_secs(5),
        }
    }

    pub fn with_limits(max_cpu_percent: f32, max_memory_mb: u64) -> Self {
        Self {
            max_cpu_percent,
            max_memory_mb,
            stats_timeout: Duration::from_secs(5),
        }
    }

    /// Check if a process is within resource limits
    /// Returns true if healthy, false if exceeding limits
    pub async fn check_process_health(&self, pid: u32) -> Result<ProcessHealth> {
        // Get process stats with timeout
        let stats_future = self.get_process_stats(pid);
        let stats = tokio::time::timeout(self.stats_timeout, stats_future).await??;

        let cpu_healthy = stats.cpu_percent <= self.max_cpu_percent;
        let memory_healthy = stats.memory_mb <= self.max_memory_mb;

        if !cpu_healthy {
            warn!(
                "Process {} exceeding CPU limit: {:.1}% > {:.1}%",
                pid, stats.cpu_percent, self.max_cpu_percent
            );
        }

        if !memory_healthy {
            warn!(
                "Process {} exceeding memory limit: {}MB > {}MB",
                pid, stats.memory_mb, self.max_memory_mb
            );
        }

        // Log warnings if approaching limits (80% of max)
        let cpu_warning_threshold = self.max_cpu_percent * 0.8;
        let memory_warning_threshold = self.max_memory_mb as f32 * 0.8;

        if stats.cpu_percent > cpu_warning_threshold && cpu_healthy {
            warn!(
                "Process {} approaching CPU limit: {:.1}% (warning at {:.1}%)",
                pid, stats.cpu_percent, cpu_warning_threshold
            );
        }

        if stats.memory_mb as f32 > memory_warning_threshold && memory_healthy {
            warn!(
                "Process {} approaching memory limit: {}MB (warning at {:.0}MB)",
                pid, stats.memory_mb, memory_warning_threshold
            );
        }

        Ok(ProcessHealth {
            pid,
            healthy: cpu_healthy && memory_healthy,
            stats,
            exceeds_cpu_limit: !cpu_healthy,
            exceeds_memory_limit: !memory_healthy,
        })
    }

    /// Monitor all child processes and return PIDs that should be killed
    pub async fn monitor_children(&self, pids: Vec<u32>) -> Vec<u32> {
        let mut unhealthy_pids = Vec::new();

        for pid in pids {
            match self.check_process_health(pid).await {
                Ok(health) => {
                    if !health.healthy {
                        warn!(
                            "Process {} is unhealthy - CPU: {:.1}% (max: {:.1}%), Memory: {}MB (max: {}MB)",
                            pid,
                            health.stats.cpu_percent,
                            self.max_cpu_percent,
                            health.stats.memory_mb,
                            self.max_memory_mb
                        );
                        unhealthy_pids.push(pid);
                    } else {
                        debug!(
                            "Process {} healthy - CPU: {:.1}%, Memory: {}MB",
                            pid, health.stats.cpu_percent, health.stats.memory_mb
                        );
                    }
                }
                Err(e) => {
                    // Process might have died or we can't access it
                    debug!("Could not check health for process {}: {}", pid, e);
                    // Don't add to unhealthy_pids as the process might be legitimately gone
                }
            }
        }

        unhealthy_pids
    }

    /// Get process statistics
    async fn get_process_stats(&self, pid: u32) -> Result<ProcessStats> {
        // Use procfs on Linux/Unix or similar approach
        #[cfg(target_os = "linux")]
        {
            self.get_process_stats_linux(pid).await
        }
        #[cfg(target_os = "macos")]
        {
            self.get_process_stats_macos(pid).await
        }
        #[cfg(target_os = "windows")]
        {
            self.get_process_stats_windows(pid).await
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            // Fallback for other systems - return default values
            warn!("Process monitoring not implemented for this platform");
            Ok(ProcessStats {
                pid,
                cpu_percent: 0.0,
                memory_mb: 0,
                running: true,
            })
        }
    }

    #[cfg(target_os = "linux")]
    async fn get_process_stats_linux(&self, pid: u32) -> Result<ProcessStats> {
        use std::fs;

        // Read /proc/{pid}/stat for CPU and memory info
        let stat_path = format!("/proc/{}/stat", pid);
        let stat_content = fs::read_to_string(&stat_path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", stat_path, e))?;

        // Parse stat file (fields are space-separated)
        let fields: Vec<&str> = stat_content.split_whitespace().collect();
        if fields.len() < 24 {
            return Err(anyhow::anyhow!("Invalid stat file format for PID {}", pid));
        }

        // Get RSS (Resident Set Size) in pages - field 23 (0-indexed)
        let rss_pages: u64 = fields[23].parse().unwrap_or(0);
        let page_size = 4096; // Standard page size on Linux
        let memory_bytes = rss_pages * page_size;
        let memory_mb = memory_bytes / (1024 * 1024);

        // For CPU, we'd need to compare with previous readings
        // For simplicity, we'll use a basic approach with /proc/{pid}/status
        let status_path = format!("/proc/{}/status", pid);
        let cpu_percent = if let Ok(_status_content) = fs::read_to_string(&status_path) {
            // Look for VmSize or other indicators
            // This is simplified - in practice, you'd want to track CPU time over intervals
            0.0 // Placeholder - real CPU monitoring requires time sampling
        } else {
            0.0
        };

        Ok(ProcessStats {
            pid,
            cpu_percent,
            memory_mb,
            running: true,
        })
    }

    #[cfg(target_os = "macos")]
    async fn get_process_stats_macos(&self, pid: u32) -> Result<ProcessStats> {
        use std::process::Command;

        // Use ps command to get process stats
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "pid,pcpu,rss"])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run ps command: {}", e))?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Process {} not found", pid));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();

        if lines.len() < 2 {
            return Err(anyhow::anyhow!("Invalid ps output for PID {}", pid));
        }

        // Parse the data line (skip header)
        let data_line = lines[1];
        let fields: Vec<&str> = data_line.split_whitespace().collect();

        if fields.len() < 3 {
            return Err(anyhow::anyhow!("Invalid ps output format for PID {}", pid));
        }

        let cpu_percent: f32 = fields[1].parse().unwrap_or(0.0);
        let memory_kb: u64 = fields[2].parse().unwrap_or(0);
        let memory_mb = memory_kb / 1024;

        Ok(ProcessStats {
            pid,
            cpu_percent,
            memory_mb,
            running: true,
        })
    }

    #[cfg(target_os = "windows")]
    async fn get_process_stats_windows(&self, pid: u32) -> Result<ProcessStats> {
        // On Windows, we'd use WMI or Windows API calls
        // This is a simplified placeholder
        warn!("Windows process monitoring not fully implemented");
        Ok(ProcessStats {
            pid,
            cpu_percent: 0.0,
            memory_mb: 0,
            running: true,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProcessStats {
    pub pid: u32,
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub running: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessHealth {
    pub pid: u32,
    pub healthy: bool,
    pub stats: ProcessStats,
    pub exceeds_cpu_limit: bool,
    pub exceeds_memory_limit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_watchdog_creation() {
        let watchdog = Watchdog::new(30);
        // Initially, no heartbeat has been set, so is_healthy() returns true since age is 0
        // which is <= timeout. We only consider it unhealthy if it has an actual age > timeout
        assert!(watchdog.is_healthy()); // age is 0, which is <= timeout
        assert_eq!(watchdog.get_heartbeat_age(), 0);
    }

    #[test]
    fn test_watchdog_heartbeat() {
        let watchdog = Watchdog::new(30);
        watchdog.heartbeat();
        assert!(watchdog.is_healthy());
        assert!(watchdog.get_heartbeat_age() <= 1); // Should be very recent
    }

    #[tokio::test]
    async fn test_watchdog_timeout() {
        let watchdog = Watchdog::new(1); // 1 second timeout

        // Set initial heartbeat
        watchdog.heartbeat();
        assert!(watchdog.is_healthy());

        // Wait for timeout
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!watchdog.is_healthy());
        assert!(watchdog.get_heartbeat_age() >= 2);
    }

    #[test]
    fn test_process_monitor_creation() {
        let monitor = ProcessMonitor::new();
        assert_eq!(monitor.max_cpu_percent, 80.0);
        assert_eq!(monitor.max_memory_mb, 1024);

        let custom_monitor = ProcessMonitor::with_limits(50.0, 512);
        assert_eq!(custom_monitor.max_cpu_percent, 50.0);
        assert_eq!(custom_monitor.max_memory_mb, 512);
    }

    #[test]
    fn test_process_stats() {
        let stats = ProcessStats {
            pid: 1234,
            cpu_percent: 25.5,
            memory_mb: 256,
            running: true,
        };

        assert_eq!(stats.pid, 1234);
        assert_eq!(stats.cpu_percent, 25.5);
        assert_eq!(stats.memory_mb, 256);
        assert!(stats.running);
    }

    #[test]
    fn test_process_health() {
        let stats = ProcessStats {
            pid: 1234,
            cpu_percent: 90.0, // High CPU
            memory_mb: 256,
            running: true,
        };

        let health = ProcessHealth {
            pid: 1234,
            healthy: false, // Due to high CPU
            stats,
            exceeds_cpu_limit: true,
            exceeds_memory_limit: false,
        };

        assert!(!health.healthy);
        assert!(health.exceeds_cpu_limit);
        assert!(!health.exceeds_memory_limit);
    }

    #[tokio::test]
    async fn test_watchdog_recovery_callback() {
        let watchdog = Watchdog::new(60);
        let recovery_called = Arc::new(AtomicBool::new(false));
        let recovery_called_clone = recovery_called.clone();

        watchdog
            .set_recovery_callback(move || {
                recovery_called_clone.store(true, Ordering::Relaxed);
            })
            .await;

        // Verify callback is set (we can't easily test the actual callback without
        // waiting for timeout in a real scenario)
        // In a real implementation, you might expose a method to trigger recovery for testing
    }
}
