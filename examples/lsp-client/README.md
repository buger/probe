# LSP Client Example

A reference implementation of an LSP client that uses the lsp-daemon for multi-language code intelligence.

## Overview

This example demonstrates how to build a client that communicates with the LSP daemon to get code intelligence features like call hierarchy, definitions, and references across 20+ programming languages.

## Features

- Automatic daemon spawning if not running
- Fallback to direct LSP mode if daemon fails
- Support for all languages configured in lsp-daemon
- Simple CLI interface for testing

## Usage

```bash
# Build the example
cargo build --release -p lsp-client

# Basic usage - analyze a file
./target/release/lsp-client main.rs "fn main"

# Daemon management commands
./target/release/lsp-client status      # Check daemon status
./target/release/lsp-client languages   # List available LSP servers
./target/release/lsp-client ping        # Health check
./target/release/lsp-client shutdown    # Shutdown daemon

# Force direct mode (no daemon)
./target/release/lsp-client --no-daemon file.rs "pattern"
```

## Implementation Details

The client consists of two main components:

### 1. LspClient (daemon mode)
- Connects to the daemon via IPC (Unix socket or Windows named pipe)
- Auto-starts daemon if not running
- Sends requests using the daemon protocol
- Handles responses and errors

### 2. DirectLspClient (fallback mode)
- Spawns LSP servers directly
- Manages server lifecycle
- Used when daemon is unavailable or disabled

## Code Structure

```rust
// Main client implementation
pub struct LspClient {
    stream: Option<IpcStream>,
    auto_start_daemon: bool,
}

impl LspClient {
    // Connect to daemon
    pub async fn new(auto_start: bool) -> Result<Self>
    
    // Send call hierarchy request
    pub async fn call_hierarchy(&mut self, file_path: &Path, pattern: &str) -> Result<CallHierarchyResult>
    
    // Get daemon status
    pub async fn get_status(&mut self) -> Result<DaemonStatus>
    
    // List available languages
    pub async fn list_languages(&mut self) -> Result<Vec<LanguageInfo>>
}
```

## Building Your Own Client

To build your own client, add lsp-daemon as a dependency:

```toml
[dependencies]
lsp-daemon = { path = "../../lsp-daemon" }  # or version when published
```

Then use the provided types and functions:

```rust
use lsp_daemon::{
    IpcStream, 
    DaemonRequest, 
    DaemonResponse,
    MessageCodec,
    get_default_socket_path,
    start_daemon_background,
};

// Connect to daemon
let mut stream = IpcStream::connect(&get_default_socket_path()).await?;

// Send request
let request = DaemonRequest::Ping { request_id: Uuid::new_v4() };
let encoded = MessageCodec::encode(&request)?;
stream.write_all(&encoded).await?;

// Read response
// ... (see full example in src/client.rs)
```

## Error Handling

The client includes comprehensive error handling:
- Connection failures trigger daemon auto-start
- Daemon failures fall back to direct mode
- Timeout protection for all operations
- Graceful degradation when LSP servers are unavailable

## Testing

Run the test suite:

```bash
cargo test -p lsp-client
```

Test with different languages:

```bash
# Rust
./target/release/lsp-client src/main.rs "fn main"

# Python
./target/release/lsp-client script.py "def process"

# TypeScript
./target/release/lsp-client app.ts "class App"

# Go
./target/release/lsp-client main.go "func main"
```

## Manual Testing Checklist

Use this checklist to verify the daemon and client are working correctly:

### 1. Basic Daemon Operations
- [ ] **Clean Start**: Kill any existing daemon process
  ```bash
  pkill -f lsp-daemon
  ```

- [ ] **Auto-Start Test**: Verify daemon starts automatically
  ```bash
  ./target/release/lsp-client ping
  # Should show: "Starting daemon..." then "Daemon is responsive"
  ```

- [ ] **Connection Test**: Verify reconnection to existing daemon
  ```bash
  ./target/release/lsp-client ping
  # Should show: "Connected to existing daemon" (no startup message)
  ```

- [ ] **Status Check**: Verify daemon status reporting
  ```bash
  ./target/release/lsp-client status
  # Should show uptime, request count, and pool status
  ```

### 2. Language Support
- [ ] **List Languages**: Check available LSP servers
  ```bash
  ./target/release/lsp-client languages
  # Should list all configured languages with availability status
  ```

- [ ] **Test Installed LSP**: Verify LSP servers work (requires LSP installed)
  ```bash
  # Create test file
  echo 'fn main() { println!("test"); }' > /tmp/test.rs
  ./target/release/lsp-client /tmp/test.rs "fn main"
  # Should return call hierarchy information
  ```

### 3. Error Handling
- [ ] **Daemon Failure**: Test fallback to direct mode
  ```bash
  # Kill daemon
  pkill -f lsp-daemon
  # Immediately test with --no-daemon flag
  ./target/release/lsp-client --no-daemon /tmp/test.rs "fn main"
  # Should work without daemon
  ```

- [ ] **Invalid File**: Test error handling for non-existent files
  ```bash
  ./target/release/lsp-client /nonexistent/file.rs "pattern"
  # Should show appropriate error message
  ```

- [ ] **Unknown Language**: Test with unsupported file type
  ```bash
  echo "test" > /tmp/test.xyz
  ./target/release/lsp-client /tmp/test.xyz "test"
  # Should report unknown language error
  ```

### 4. Performance Testing
- [ ] **Cold Start**: Time first request after daemon start
  ```bash
  pkill -f lsp-daemon
  time ./target/release/lsp-client ping
  # Should complete in ~100-200ms
  ```

- [ ] **Warm Request**: Time subsequent requests
  ```bash
  time ./target/release/lsp-client ping
  # Should complete in ~10-50ms
  ```

- [ ] **Multiple Connections**: Test concurrent connections
  ```bash
  for i in {1..5}; do
    ./target/release/lsp-client ping &
  done
  wait
  # All should succeed
  ```

### 5. Daemon Management
- [ ] **Graceful Shutdown**: Test daemon shutdown
  ```bash
  ./target/release/lsp-client shutdown
  # Should show: "Daemon shutdown complete"
  ```

- [ ] **Process Cleanup**: Verify daemon process is gone
  ```bash
  ps aux | grep lsp-daemon | grep -v grep
  # Should return nothing
  ```

- [ ] **Socket Cleanup**: Verify socket file is cleaned up
  ```bash
  ls -la /tmp/lsp-daemon.sock
  # Should not exist after shutdown
  ```

### 6. Cross-Platform Testing (if applicable)
- [ ] **Unix Socket** (Linux/macOS): Verify socket creation
  ```bash
  ./target/release/lsp-daemon --foreground &
  ls -la /tmp/lsp-daemon.sock
  # Should show socket file
  ```

- [ ] **Named Pipe** (Windows): Verify pipe creation
  ```powershell
  # On Windows
  .\target\release\lsp-daemon.exe --foreground &
  Get-ChildItem \\.\pipe\ | Select-String lsp-daemon
  # Should show named pipe
  ```

### 7. Long-Running Test
- [ ] **24-Hour Idle**: Verify daemon stays alive for 24 hours
  ```bash
  ./target/release/lsp-daemon --foreground &
  # Leave running and check after 24 hours
  # Should auto-shutdown after 24 hours of inactivity
  ```

### Expected Results Summary
✅ All commands should complete without errors
✅ Daemon should auto-start within 200ms
✅ Subsequent requests should complete within 50ms
✅ Fallback to direct mode should work seamlessly
✅ All cleanup should happen automatically
✅ Socket/pipe files should be managed correctly

## Known Issues and Workarounds

### gopls (Go Language Server) Performance

The Go language server (gopls) can be extremely slow to initialize (30-60 seconds) when no `go.mod` file is present. This happens because gopls attempts to scan the entire filesystem looking for Go modules, including your home directory and system folders.

**Symptoms:**
- gopls uses 100%+ CPU during startup
- Requests timeout after 30-60 seconds
- Multiple gopls processes may spawn

**Root Cause:**
When gopls doesn't find a `go.mod` file, it runs `findModules` which recursively scans directories. On macOS, this includes the `~/Library` folder which can contain hundreds of thousands of files.

**Implemented Fixes:**
1. Increased gopls initialization timeout to 60 seconds
2. Added initialization options to limit gopls scope:
   - `directoryFilters`: Restricts scanning to current directory only
   - `expandWorkspaceToModule`: Disabled to prevent full module scanning
   - `symbolScope`: Limited to workspace only
3. gopls starts in `/tmp` directory to avoid home directory scanning
4. Added spawning lock to prevent multiple gopls instances

**User Workarounds:**
1. **Always use go.mod files**: Create a `go.mod` file in your Go projects:
   ```bash
   go mod init myproject
   ```

2. **Use go.work files**: For multiple modules, create a `go.work` file:
   ```bash
   go work init
   go work use ./module1 ./module2
   ```

3. **Test in isolated directories**: When testing, use a directory with go.mod:
   ```bash
   mkdir /tmp/gotest && cd /tmp/gotest
   go mod init test
   # Now gopls will start quickly
   ```

### Other Language Servers

Some language servers may also have slow initialization times:
- **Scala (metals)**: 60 seconds timeout configured
- **Java (jdtls)**: 45 seconds timeout configured
- **Kotlin**: 45 seconds timeout configured

These servers typically need to index dependencies and build artifacts on first run.

## License

MIT - See LICENSE file in the repository root