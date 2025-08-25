# LSP Daemon Stress Tests

This directory contains comprehensive stress tests that validate the robustness of the LSP daemon under various failure scenarios.

## Running the Stress Tests

The stress tests are marked with `#[ignore]` since they are long-running and resource-intensive. Run them with:

```bash
# Run all stress tests
cargo test --test stress_tests -- --ignored

# Run a specific stress test
cargo test test_daemon_handles_unresponsive_client --test stress_tests -- --ignored

# Run the mock server infrastructure test (not ignored)
cargo test test_mock_lsp_server_functionality --test stress_tests
```

## Test Categories

### 1. Connection Handling Tests

- **`test_daemon_handles_unresponsive_client`**: Validates that the daemon can handle clients that send partial messages and become unresponsive
- **`test_daemon_handles_many_concurrent_connections`**: Tests connection limit enforcement and graceful rejection of excess connections
- **`test_connection_cleanup_prevents_resource_leak`**: Verifies that idle connections are properly cleaned up to prevent memory leaks

### 2. Failure Recovery Tests

- **`test_health_monitor_restarts_unhealthy_servers`**: Tests health monitoring and automatic server restart capabilities
- **`test_circuit_breaker_prevents_cascading_failures`**: Validates circuit breaker functionality to prevent cascading failures
- **`test_daemon_handles_lsp_server_crash`**: Tests graceful handling of LSP server process crashes

### 3. System Monitoring Tests

- **`test_watchdog_detects_unresponsive_daemon`**: Validates watchdog mechanism for detecting unresponsive daemon processes
- **`test_daemon_stability_over_time`**: Long-running stability test that simulates extended operation with periodic requests

### 4. Message Handling Tests

- **`test_daemon_handles_large_messages`**: Tests handling of progressively larger messages (1KB to 1MB)

## Mock LSP Server Infrastructure

The tests include a comprehensive mock LSP server (`MockLspServer`) that can simulate various failure modes:

- **Normal**: Standard LSP server behavior
- **SlowResponses**: Delayed responses to test timeout handling  
- **FailAfterN**: Fails after a specified number of requests
- **RandomFailures**: Fails with a configurable probability
- **MemoryLeak**: Intentionally leaks memory to test resource monitoring
- **Unresponsive**: Never responds to requests
- **PartialResponses**: Sends incomplete responses
- **InvalidJson**: Sends malformed JSON responses

## Test Infrastructure

### Memory Monitoring

The tests include platform-specific memory usage monitoring:

- **Linux**: Uses `/proc/self/status` 
- **macOS**: Uses `proc_pidinfo` system call
- **Other platforms**: Fallback implementation

### Performance Metrics

Each test tracks relevant metrics:

- Request/response latencies
- Memory usage over time
- Connection counts
- Error rates
- Throughput measurements

### Cleanup and Safety

All tests include proper cleanup mechanisms:

- Automatic daemon shutdown
- Socket file removal
- Resource deallocation
- Graceful test termination

## Running Individual Tests

```bash
# Test unresponsive client handling
cargo test test_daemon_handles_unresponsive_client --test stress_tests -- --ignored

# Test concurrent connections
cargo test test_daemon_handles_many_concurrent_connections --test stress_tests -- --ignored

# Test health monitoring (requires LSP server)
cargo test test_health_monitor_restarts_unhealthy_servers --test stress_tests -- --ignored

# Test circuit breaker functionality
cargo test test_circuit_breaker_prevents_cascading_failures --test stress_tests -- --ignored

# Test watchdog mechanism
cargo test test_watchdog_detects_unresponsive_daemon --test stress_tests -- --ignored

# Test connection cleanup
cargo test test_connection_cleanup_prevents_resource_leak --test stress_tests -- --ignored

# Test LSP server crash handling
cargo test test_daemon_handles_lsp_server_crash --test stress_tests -- --ignored

# Test long-term stability (shortened for testing)
cargo test test_daemon_stability_over_time --test stress_tests -- --ignored

# Test large message handling
cargo test test_daemon_handles_large_messages --test stress_tests -- --ignored
```

## Expected Test Durations

- **Short tests** (< 30 seconds): `test_daemon_handles_unresponsive_client`, `test_watchdog_detects_unresponsive_daemon`
- **Medium tests** (30-60 seconds): `test_daemon_handles_many_concurrent_connections`, `test_circuit_breaker_prevents_cascading_failures`
- **Long tests** (1-5 minutes): `test_health_monitor_restarts_unhealthy_servers`, `test_connection_cleanup_prevents_resource_leak`, `test_daemon_stability_over_time`

## Test Requirements

- **Unix sockets**: Tests require Unix domain socket support (Linux/macOS)
- **Memory**: Some tests require sufficient memory for connection pools
- **File descriptors**: Concurrent connection tests may require increased fd limits
- **Time**: Long-running tests simulate extended daemon operation

## Interpreting Results

### Success Criteria

- All connections are handled gracefully
- Memory usage remains within acceptable bounds
- Error rates stay below 10%
- Recovery mechanisms activate when needed
- No resource leaks detected

### Common Failure Modes

- **Connection timeouts**: May indicate insufficient system resources
- **Memory growth**: Could signal resource leaks needing investigation
- **High error rates**: May indicate insufficient error handling
- **Test hangs**: Could indicate deadlocks or infinite loops

## Integration with CI

For continuous integration, run a subset of faster tests:

```bash
# Run only infrastructure and short stress tests
cargo test test_mock_lsp_server_functionality --test stress_tests
cargo test test_daemon_handles_unresponsive_client --test stress_tests -- --ignored
cargo test test_watchdog_detects_unresponsive_daemon --test stress_tests -- --ignored
```

Full stress testing should be performed during release validation or scheduled maintenance windows.