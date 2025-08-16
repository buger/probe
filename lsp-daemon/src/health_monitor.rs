use crate::language_detector::Language;
use crate::server_manager::SingleServerManager;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub struct ServerHealth {
    pub consecutive_failures: u32,
    pub last_check: Instant,
    pub is_healthy: bool,
    pub response_time_ms: u64,
    pub last_success: Option<Instant>,
    pub circuit_breaker_open_until: Option<Instant>,
}

impl Default for ServerHealth {
    fn default() -> Self {
        Self {
            consecutive_failures: 0,
            last_check: Instant::now(),
            is_healthy: true,
            response_time_ms: 0,
            last_success: Some(Instant::now()),
            circuit_breaker_open_until: None,
        }
    }
}

impl ServerHealth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_success(&mut self, response_time_ms: u64) {
        self.consecutive_failures = 0;
        self.is_healthy = true;
        self.response_time_ms = response_time_ms;
        self.last_success = Some(Instant::now());
        self.last_check = Instant::now();
        self.circuit_breaker_open_until = None;
    }

    pub fn mark_failure(&mut self) {
        self.consecutive_failures += 1;
        self.is_healthy = false;
        self.last_check = Instant::now();

        // Implement exponential backoff for circuit breaker
        if self.consecutive_failures >= 3 {
            let backoff_seconds = 10u64.pow(std::cmp::min(self.consecutive_failures - 3, 3)); // 10s, 100s, 1000s max
            self.circuit_breaker_open_until =
                Some(Instant::now() + Duration::from_secs(backoff_seconds));
            debug!(
                "Circuit breaker opened for {}s after {} consecutive failures",
                backoff_seconds, self.consecutive_failures
            );
        }
    }

    pub fn is_circuit_breaker_open(&self) -> bool {
        if let Some(open_until) = self.circuit_breaker_open_until {
            Instant::now() < open_until
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub struct HealthMonitor {
    health_status: Arc<Mutex<HashMap<String, ServerHealth>>>, // key: language_workspace
    failure_threshold: u32,
    check_interval: Duration,
    health_check_timeout: Duration,
}

impl HealthMonitor {
    pub fn new() -> Self {
        Self {
            health_status: Arc::new(Mutex::new(HashMap::new())),
            failure_threshold: 3,
            check_interval: Duration::from_secs(30),
            health_check_timeout: Duration::from_secs(5),
        }
    }

    pub fn with_config(
        failure_threshold: u32,
        check_interval: Duration,
        health_check_timeout: Duration,
    ) -> Self {
        Self {
            health_status: Arc::new(Mutex::new(HashMap::new())),
            failure_threshold,
            check_interval,
            health_check_timeout,
        }
    }

    /// Start background health check task
    pub fn start_monitoring(
        &self,
        server_manager: Arc<SingleServerManager>,
    ) -> tokio::task::JoinHandle<()> {
        let health_status = self.health_status.clone();
        let check_interval = self.check_interval;
        let health_check_timeout = self.health_check_timeout;

        tokio::spawn(async move {
            let mut interval_timer = interval(check_interval);

            loop {
                interval_timer.tick().await;

                debug!("Running periodic health checks");

                // Get list of active servers
                let server_stats = server_manager.get_stats().await;

                for stat in server_stats {
                    if !stat.initialized {
                        continue; // Skip uninitialized servers
                    }

                    // Generate a unique key for this server
                    let server_key = format!("{:?}", stat.language);

                    // Perform health check with timeout
                    let health_result = tokio::time::timeout(
                        health_check_timeout,
                        Self::perform_health_check_internal(&server_manager, stat.language),
                    )
                    .await;

                    let mut health_map = health_status.lock().await;
                    let server_health = health_map.entry(server_key.clone()).or_default();

                    match health_result {
                        Ok(Ok(response_time)) => {
                            debug!(
                                "Health check passed for {:?} ({}ms)",
                                stat.language, response_time
                            );
                            server_health.mark_success(response_time);
                        }
                        Ok(Err(e)) => {
                            warn!("Health check failed for {:?}: {}", stat.language, e);
                            server_health.mark_failure();
                        }
                        Err(_) => {
                            warn!("Health check timed out for {:?}", stat.language);
                            server_health.mark_failure();
                        }
                    }

                    // Check if server needs restart
                    if server_health.consecutive_failures >= 3 {
                        error!(
                            "Server {:?} has {} consecutive failures, restart needed",
                            stat.language, server_health.consecutive_failures
                        );

                        // TODO: Implement server restart logic
                        // This would involve shutting down the unhealthy server and
                        // letting the server manager create a new one on next request
                    }
                }
            }
        })
    }

    /// Perform health check on a specific server
    pub async fn check_server_health(&self, language: Language) -> Result<bool> {
        let server_key = format!("{language:?}");

        // Check circuit breaker first
        {
            let health_map = self.health_status.lock().await;
            if let Some(health) = health_map.get(&server_key) {
                if health.is_circuit_breaker_open() {
                    debug!("Circuit breaker is open for {:?}, failing fast", language);
                    return Ok(false);
                }
            }
        }

        // For now, return true - actual health check would need server manager integration
        Ok(true)
    }

    /// Internal method to perform health check by sending a simple LSP request
    async fn perform_health_check_internal(
        server_manager: &SingleServerManager,
        language: Language,
    ) -> Result<u64> {
        let start = Instant::now();

        // Try to get the server instance
        let server_instance = server_manager.get_server(language).await?;

        // Try to acquire lock with short timeout (use try_lock to avoid blocking)
        let _server = match server_instance.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // If we can't acquire the lock immediately, the server might be busy
                // This is not necessarily unhealthy, just busy
                debug!(
                    "Server {:?} is busy, skipping detailed health check",
                    language
                );
                // Return a nominal response time to indicate the server exists but is busy
                return Ok(1);
            }
        };

        // For servers that support workspace/symbol requests, use that as health check
        // Otherwise, we just check that we can acquire the lock (server is responsive)
        match language {
            Language::Rust | Language::TypeScript | Language::Python | Language::Go => {
                // For these languages, just check that we successfully acquired the lock
                // Don't send additional requests during health check to avoid interfering
                // with ongoing operations
                let elapsed = start.elapsed();
                Ok(elapsed.as_millis() as u64)
            }
            _ => {
                // For other languages, just check that server is responsive (lock acquired)
                let elapsed = start.elapsed();
                Ok(elapsed.as_millis() as u64)
            }
        }
    }

    /// Mark a server as unhealthy (called when operations fail)
    pub async fn mark_unhealthy(&self, language: Language) {
        let server_key = format!("{language:?}");
        let mut health_map = self.health_status.lock().await;
        let server_health = health_map.entry(server_key.clone()).or_default();

        server_health.mark_failure();

        debug!(
            "Marked {:?} as unhealthy (consecutive failures: {})",
            language, server_health.consecutive_failures
        );
    }

    /// Mark a server as healthy (called when operations succeed)
    pub async fn mark_healthy(&self, language: Language) {
        let server_key = format!("{language:?}");
        let mut health_map = self.health_status.lock().await;
        let server_health = health_map.entry(server_key.clone()).or_default();

        let response_time = 0; // We don't measure response time for successful operations
        server_health.mark_success(response_time);

        debug!("Marked {:?} as healthy", language);
    }

    /// Check if a server should be restarted
    pub async fn should_restart(&self, language: Language) -> bool {
        let server_key = format!("{language:?}");
        let health_map = self.health_status.lock().await;

        if let Some(health) = health_map.get(&server_key) {
            health.consecutive_failures >= self.failure_threshold
        } else {
            false
        }
    }

    /// Check if requests to a server should be rejected (circuit breaker)
    pub async fn should_reject_request(&self, language: Language) -> bool {
        let server_key = format!("{language:?}");
        let health_map = self.health_status.lock().await;

        if let Some(health) = health_map.get(&server_key) {
            health.is_circuit_breaker_open()
        } else {
            false
        }
    }

    /// Get health status for all servers
    pub async fn get_health_status(&self) -> HashMap<String, ServerHealth> {
        let health_map = self.health_status.lock().await;
        health_map.clone()
    }

    /// Reset health status for a specific server (useful after manual restart)
    pub async fn reset_health_status(&self, language: Language) {
        let server_key = format!("{language:?}");
        let mut health_map = self.health_status.lock().await;
        health_map.insert(server_key, ServerHealth::new());

        info!("Reset health status for {:?}", language);
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_server_health_success() {
        let mut health = ServerHealth::new();

        // Initially healthy
        assert!(health.is_healthy);
        assert_eq!(health.consecutive_failures, 0);
        assert!(!health.is_circuit_breaker_open());

        // Mark as successful
        health.mark_success(100);
        assert!(health.is_healthy);
        assert_eq!(health.consecutive_failures, 0);
        assert_eq!(health.response_time_ms, 100);
        assert!(!health.is_circuit_breaker_open());
    }

    #[test]
    fn test_server_health_failure() {
        let mut health = ServerHealth::new();

        // Mark first failure
        health.mark_failure();
        assert!(!health.is_healthy);
        assert_eq!(health.consecutive_failures, 1);
        assert!(!health.is_circuit_breaker_open());

        // Mark second failure
        health.mark_failure();
        assert!(!health.is_healthy);
        assert_eq!(health.consecutive_failures, 2);
        assert!(!health.is_circuit_breaker_open());

        // Mark third failure - should open circuit breaker
        health.mark_failure();
        assert!(!health.is_healthy);
        assert_eq!(health.consecutive_failures, 3);
        assert!(health.is_circuit_breaker_open());
    }

    #[test]
    fn test_circuit_breaker_recovery() {
        let mut health = ServerHealth::new();

        // Trigger circuit breaker
        health.mark_failure();
        health.mark_failure();
        health.mark_failure();
        assert!(health.is_circuit_breaker_open());

        // Recovery should reset circuit breaker
        health.mark_success(50);
        assert!(health.is_healthy);
        assert_eq!(health.consecutive_failures, 0);
        assert!(!health.is_circuit_breaker_open());
    }

    #[tokio::test]
    async fn test_health_monitor_creation() {
        let monitor = HealthMonitor::new();
        assert_eq!(monitor.failure_threshold, 3);
        assert_eq!(monitor.check_interval, Duration::from_secs(30));

        let status = monitor.get_health_status().await;
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn test_health_monitor_mark_operations() {
        let monitor = HealthMonitor::new();
        let language = Language::Rust;

        // Mark as unhealthy
        monitor.mark_unhealthy(language).await;
        let status = monitor.get_health_status().await;
        let server_health = status.get("Rust").unwrap();
        assert!(!server_health.is_healthy);
        assert_eq!(server_health.consecutive_failures, 1);

        // Mark as healthy
        monitor.mark_healthy(language).await;
        let status = monitor.get_health_status().await;
        let server_health = status.get("Rust").unwrap();
        assert!(server_health.is_healthy);
        assert_eq!(server_health.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn test_should_restart_logic() {
        let monitor = HealthMonitor::new();
        let language = Language::Rust;

        // Initially should not restart
        assert!(!monitor.should_restart(language).await);

        // Mark as unhealthy multiple times
        monitor.mark_unhealthy(language).await;
        monitor.mark_unhealthy(language).await;
        assert!(!monitor.should_restart(language).await);

        // Third failure should trigger restart
        monitor.mark_unhealthy(language).await;
        assert!(monitor.should_restart(language).await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_rejection() {
        let monitor = HealthMonitor::new();
        let language = Language::Rust;

        // Initially should not reject
        assert!(!monitor.should_reject_request(language).await);

        // Trigger circuit breaker
        monitor.mark_unhealthy(language).await;
        monitor.mark_unhealthy(language).await;
        monitor.mark_unhealthy(language).await;

        // Should reject requests
        assert!(monitor.should_reject_request(language).await);

        // Recovery should stop rejecting
        monitor.mark_healthy(language).await;
        assert!(!monitor.should_reject_request(language).await);
    }
}
