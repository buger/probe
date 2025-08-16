# LSP Client Implementation Guide

This guide describes how to implement a client that communicates with the probe LSP daemon.

## Architecture Overview

The LSP integration uses a daemon-based architecture:

```
CLI Client → IPC Socket → LSP Daemon → Server Manager → Language Servers
                              ↓
                        In-Memory Log Buffer (1000 entries)
```

**Key Components:**
- **LSP Daemon**: Persistent background service at `lsp-daemon/src/daemon.rs`
- **Server Manager**: Pool management at `lsp-daemon/src/server_manager.rs`
- **LSP Client**: IPC communication at `src/lsp_integration/client.rs`
- **Protocol Layer**: Request/response types at `lsp-daemon/src/protocol.rs`
- **Logging System**: In-memory circular buffer at `lsp-daemon/src/logging.rs`

## Finding the Socket Path

The daemon uses a platform-specific socket location:

```rust
// Unix/macOS
fn get_default_socket_path() -> String {
    let temp_dir = std::env::var("TMPDIR")
        .unwrap_or_else(|_| "/tmp".to_string());
    format!("{}/lsp-daemon.sock", temp_dir)
}

// Windows
fn get_default_socket_path() -> String {
    r"\\.\pipe\lsp-daemon".to_string()
}
```

**Example paths:**
- macOS: `/var/folders/bd/7mkdqnbs13x30zb67bm7xrm00000gn/T/lsp-daemon.sock`
- Linux: `/tmp/lsp-daemon.sock`
- Windows: `\\.\pipe\lsp-daemon`

## Wire Protocol

The daemon uses a **length-prefixed binary protocol** with JSON serialization:

```
[4 bytes: message length (big-endian)] [N bytes: JSON-encoded message]
```

**Message Flow:**
1. Encode request/response as JSON
2. Prepend 4-byte length header (big-endian)
3. Send over socket
4. Read 4-byte length header
5. Read N bytes of JSON data
6. Decode JSON to get message

**Important:** The JSON uses tagged enums with a `type` field (due to `#[serde(tag = "type")]`)

## Request/Response Types

All messages are strongly typed. Key types from `lsp-daemon/src/protocol.rs`:

```rust
// Note: Uses #[serde(tag = "type")] for JSON encoding
pub enum DaemonRequest {
    // Initial handshake
    Connect { client_id: Uuid },
    
    // Health check
    Ping { request_id: Uuid },
    
    // Get daemon status
    Status { request_id: Uuid },
    
    // Get call hierarchy for a symbol
    CallHierarchy {
        request_id: Uuid,
        file_path: String,
        line: u32,
        column: u32,
        workspace_hint: Option<String>,
    },
    
    // Shutdown daemon
    Shutdown { request_id: Uuid },
    
    // Get daemon logs
    GetLogs { 
        request_id: Uuid, 
        lines: usize,
    },
}

// Example JSON requests:
// Connect: {"type": "Connect", "client_id": "550e8400-e29b-41d4-a716-446655440000"}
// Status:  {"type": "Status", "request_id": "550e8400-e29b-41d4-a716-446655440000"}
// Ping:    {"type": "Ping", "request_id": "550e8400-e29b-41d4-a716-446655440000"}

#[derive(Serialize, Deserialize)]
pub enum DaemonResponse {
    Connected { 
        daemon_version: String,
        client_id: Uuid,
    },
    Pong { request_id: Uuid },
    Status { 
        request_id: Uuid,
        status: DaemonStatus,
    },
    CallHierarchy { 
        request_id: Uuid,
        result: CallHierarchyResult,
    },
    Shutdown { request_id: Uuid },
    Error { 
        request_id: Uuid,
        error: String,
    },
    Logs {
        request_id: Uuid,
        entries: Vec<LogEntry>,
    },
}
```

## Complete Client Implementation Examples

### Python Client Example

```python
import socket
import struct
import json
import uuid
import os
import time

class LspDaemonClient:
    def __init__(self):
        self.socket = None
        self.socket_path = self._get_socket_path()
    
    def _get_socket_path(self):
        """Get platform-specific socket path"""
        if os.name == 'nt':  # Windows
            return r'\\.\pipe\lsp-daemon'
        else:  # Unix/macOS
            temp_dir = os.environ.get('TMPDIR', '/tmp')
            return f"{temp_dir}/lsp-daemon.sock"
    
    def connect(self):
        """Connect to the daemon"""
        if os.name == 'nt':
            # Windows named pipe
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            # Note: Actual Windows implementation would use pywin32
        else:
            # Unix domain socket
            self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self.socket.connect(self.socket_path)
        
        # Send Connect message (using tagged enum format)
        client_id = str(uuid.uuid4())
        request = {
            "type": "Connect",
            "client_id": client_id
        }
        response = self._send_request(request)
        print(f"Connected to daemon: {response}")
        return client_id
    
    def _send_request(self, request):
        """Send request and receive response"""
        # Encode as JSON
        json_str = json.dumps(request)
        encoded = json_str.encode('utf-8')
        
        # Prepend length (4 bytes, big-endian)
        length = struct.pack('>I', len(encoded))
        
        # Send length + message
        self.socket.sendall(length + encoded)
        
        # Read response length
        length_bytes = self._recv_exact(4)
        response_length = struct.unpack('>I', length_bytes)[0]
        
        # Read response
        response_bytes = self._recv_exact(response_length)
        
        # Decode JSON
        json_str = response_bytes.decode('utf-8')
        return json.loads(json_str)
    
    def _recv_exact(self, n):
        """Receive exactly n bytes"""
        data = b''
        while len(data) < n:
            chunk = self.socket.recv(n - len(data))
            if not chunk:
                raise ConnectionError("Socket closed")
            data += chunk
        return data
    
    def get_status(self):
        """Get daemon status"""
        request = {
            "type": "Status",
            "request_id": str(uuid.uuid4())
        }
        return self._send_request(request)
    
    def get_call_hierarchy(self, file_path, line, column):
        """Get call hierarchy for a symbol"""
        request = {
            "type": "CallHierarchy",
            "request_id": str(uuid.uuid4()),
            "file_path": file_path,
            "line": line,
            "column": column,
            "workspace_hint": None
        }
        return self._send_request(request)
    
    def shutdown(self):
        """Shutdown the daemon"""
        request = {
            "type": "Shutdown",
            "request_id": str(uuid.uuid4())
        }
        response = self._send_request(request)
        self.socket.close()
        return response
    
    def close(self):
        """Close the connection"""
        if self.socket:
            self.socket.close()

# Example usage
if __name__ == "__main__":
    client = LspDaemonClient()
    try:
        # Connect to daemon
        client.connect()
        
        # Get status
        status = client.get_status()
        print(f"Daemon status: {status}")
        
        # Get call hierarchy
        result = client.get_call_hierarchy(
            "src/main.rs", 
            10,  # line
            5    # column
        )
        print(f"Call hierarchy: {result}")
        
    finally:
        client.close()
```

### Rust Client Example

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use uuid::Uuid;

// Import protocol types (or redefine them)
use lsp_daemon::protocol::{DaemonRequest, DaemonResponse};

pub struct LspClient {
    stream: UnixStream,
}

impl LspClient {
    /// Connect to the LSP daemon
    pub fn connect() -> Result<Self> {
        let socket_path = Self::get_socket_path();
        let stream = UnixStream::connect(&socket_path)?;
        
        let mut client = Self { stream };
        
        // Send initial Connect message
        let request = DaemonRequest::Connect {
            client_id: Uuid::new_v4(),
        };
        
        let response = client.send_request(request)?;
        
        match response {
            DaemonResponse::Connected { daemon_version, .. } => {
                println!("Connected to daemon v{}", daemon_version);
            }
            _ => return Err(anyhow::anyhow!("Unexpected response")),
        }
        
        Ok(client)
    }
    
    /// Get platform-specific socket path
    fn get_socket_path() -> String {
        #[cfg(unix)]
        {
            let temp_dir = std::env::var("TMPDIR")
                .unwrap_or_else(|_| "/tmp".to_string());
            format!("{}/lsp-daemon.sock", temp_dir)
        }
        
        #[cfg(windows)]
        {
            r"\\.\pipe\lsp-daemon".to_string()
        }
    }
    
    /// Send request and receive response
    fn send_request(&mut self, request: DaemonRequest) -> Result<DaemonResponse> {
        // Serialize with MessagePack
        let encoded = rmp_serde::to_vec(&request)?;
        
        // Write length header (4 bytes, big-endian)
        let length = encoded.len() as u32;
        self.stream.write_all(&length.to_be_bytes())?;
        
        // Write message
        self.stream.write_all(&encoded)?;
        self.stream.flush()?;
        
        // Read response length
        let mut length_buf = [0u8; 4];
        self.stream.read_exact(&mut length_buf)?;
        let response_length = u32::from_be_bytes(length_buf) as usize;
        
        // Read response
        let mut response_buf = vec![0u8; response_length];
        self.stream.read_exact(&mut response_buf)?;
        
        // Deserialize response
        let response = rmp_serde::from_slice(&response_buf)?;
        Ok(response)
    }
    
    /// Get daemon status
    pub fn get_status(&mut self) -> Result<DaemonStatus> {
        let request = DaemonRequest::Status {
            request_id: Uuid::new_v4(),
        };
        
        match self.send_request(request)? {
            DaemonResponse::Status { status, .. } => Ok(status),
            DaemonResponse::Error { error, .. } => {
                Err(anyhow::anyhow!("Error: {}", error))
            }
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }
    
    /// Get call hierarchy for a symbol
    pub fn get_call_hierarchy(
        &mut self,
        file_path: &str,
        line: u32,
        column: u32,
    ) -> Result<CallHierarchyResult> {
        let request = DaemonRequest::CallHierarchy {
            request_id: Uuid::new_v4(),
            file_path: file_path.to_string(),
            line,
            column,
            workspace_hint: None,
        };
        
        match self.send_request(request)? {
            DaemonResponse::CallHierarchy { result, .. } => Ok(result),
            DaemonResponse::Error { error, .. } => {
                Err(anyhow::anyhow!("Error: {}", error))
            }
            _ => Err(anyhow::anyhow!("Unexpected response")),
        }
    }
}

// Example usage
fn main() -> Result<()> {
    let mut client = LspClient::connect()?;
    
    // Get status
    let status = client.get_status()?;
    println!("Daemon uptime: {}s", status.uptime_secs);
    
    // Get call hierarchy
    let hierarchy = client.get_call_hierarchy(
        "src/main.rs",
        10,  // line
        5,   // column
    )?;
    
    println!("Found {} incoming calls", hierarchy.incoming_calls.len());
    
    Ok(())
}
```

### Node.js/TypeScript Client Example

```typescript
import net from 'net';
import msgpack from 'msgpack-lite';
import { v4 as uuidv4 } from 'uuid';
import os from 'os';
import path from 'path';

class LspDaemonClient {
    private socket: net.Socket | null = null;
    private socketPath: string;
    
    constructor() {
        this.socketPath = this.getSocketPath();
    }
    
    private getSocketPath(): string {
        if (process.platform === 'win32') {
            return '\\\\.\\pipe\\lsp-daemon';
        } else {
            const tmpDir = process.env.TMPDIR || '/tmp';
            return path.join(tmpDir, 'lsp-daemon.sock');
        }
    }
    
    async connect(): Promise<string> {
        return new Promise((resolve, reject) => {
            this.socket = net.createConnection(this.socketPath, () => {
                console.log('Connected to LSP daemon');
                
                // Send Connect message
                const clientId = uuidv4();
                const request = {
                    Connect: {
                        client_id: clientId
                    }
                };
                
                this.sendRequest(request).then(response => {
                    console.log('Handshake complete:', response);
                    resolve(clientId);
                }).catch(reject);
            });
            
            this.socket.on('error', reject);
        });
    }
    
    private sendRequest(request: any): Promise<any> {
        return new Promise((resolve, reject) => {
            if (!this.socket) {
                reject(new Error('Not connected'));
                return;
            }
            
            // Encode with MessagePack
            const encoded = msgpack.encode(request);
            
            // Create length header (4 bytes, big-endian)
            const lengthBuffer = Buffer.allocUnsafe(4);
            lengthBuffer.writeUInt32BE(encoded.length, 0);
            
            // Send length + message
            this.socket.write(Buffer.concat([lengthBuffer, encoded]));
            
            // Set up one-time response handler
            let responseLength = 0;
            let responseBuffer = Buffer.alloc(0);
            let headerReceived = false;
            
            const onData = (data: Buffer) => {
                responseBuffer = Buffer.concat([responseBuffer, data]);
                
                // Read header if not yet received
                if (!headerReceived && responseBuffer.length >= 4) {
                    responseLength = responseBuffer.readUInt32BE(0);
                    responseBuffer = responseBuffer.slice(4);
                    headerReceived = true;
                }
                
                // Check if we have full message
                if (headerReceived && responseBuffer.length >= responseLength) {
                    const message = responseBuffer.slice(0, responseLength);
                    const decoded = msgpack.decode(message);
                    
                    this.socket?.removeListener('data', onData);
                    resolve(decoded);
                }
            };
            
            this.socket.on('data', onData);
        });
    }
    
    async getStatus(): Promise<any> {
        const request = {
            Status: {
                request_id: uuidv4()
            }
        };
        return this.sendRequest(request);
    }
    
    async getCallHierarchy(
        filePath: string,
        line: number,
        column: number
    ): Promise<any> {
        const request = {
            CallHierarchy: {
                request_id: uuidv4(),
                file_path: filePath,
                line: line,
                column: column,
                workspace_hint: null
            }
        };
        return this.sendRequest(request);
    }
    
    async shutdown(): Promise<void> {
        const request = {
            Shutdown: {
                request_id: uuidv4()
            }
        };
        await this.sendRequest(request);
        this.close();
    }
    
    close(): void {
        if (this.socket) {
            this.socket.destroy();
            this.socket = null;
        }
    }
}

// Example usage
async function main() {
    const client = new LspDaemonClient();
    
    try {
        await client.connect();
        
        // Get status
        const status = await client.getStatus();
        console.log('Daemon status:', status);
        
        // Get call hierarchy
        const hierarchy = await client.getCallHierarchy(
            'src/main.rs',
            10,  // line
            5    // column
        );
        console.log('Call hierarchy:', hierarchy);
        
    } finally {
        client.close();
    }
}

main().catch(console.error);
```

## Auto-Starting the Daemon

If the daemon is not running, clients can start it:

```bash
# Check if daemon is running
if ! probe lsp status 2>/dev/null; then
    probe lsp start
    sleep 2  # Wait for daemon to be ready
fi
```

Or programmatically:

```python
def ensure_daemon_running(self):
    """Start daemon if not running"""
    try:
        self.connect()
    except (ConnectionError, FileNotFoundError):
        # Daemon not running, start it
        import subprocess
        subprocess.run(['probe', 'lsp', 'start'], check=True)
        time.sleep(2)  # Wait for startup
        self.connect()
```

## Connection Management Best Practices

1. **Connection Pooling**: Reuse connections for multiple requests
2. **Timeout Handling**: Set reasonable timeouts (default: 30s)
3. **Retry Logic**: Implement exponential backoff for connection failures
4. **Graceful Shutdown**: Always close connections properly
5. **Error Handling**: Handle daemon restarts/crashes gracefully

## Debugging LSP Issues

### CRITICAL: Avoid Rust Build Lock Contention

```bash
# WRONG - This will hang due to build lock conflicts:
# cargo run -- lsp start -f &
# cargo run -- lsp status  # <-- This hangs!

# CORRECT - Build first, then use binary:
cargo build
./target/debug/probe lsp start -f &
./target/debug/probe lsp status  # <-- This works!

# OR use the installed binary:
probe lsp status  # If probe is installed
```

### View LSP daemon logs (in-memory, no files)

```bash
probe lsp logs              # View last 50 log entries
probe lsp logs -n 100       # View last 100 entries
probe lsp logs --follow     # Follow logs in real-time (polls every 500ms)
```

### Check daemon status and server pools

```bash
probe lsp status            # Show daemon status, uptime, and server pools
probe lsp shutdown          # Stop daemon cleanly
probe lsp restart           # Restart daemon (clears in-memory logs)
```

### Debug in foreground mode

```bash
# Run daemon in foreground with debug logging
./target/debug/probe lsp start -f --log-level debug

# In another terminal, test LSP operations
./target/debug/probe extract file.rs#symbol --lsp
```

### Common LSP issues and solutions

| Issue | Cause | Solution |
|-------|-------|----------|
| **No call hierarchy data** | Language server still indexing | Wait 10-15s for rust-analyzer to index |
| **Timeout errors** | Large codebase or slow language server | Increase timeout in client config |
| **Connection refused** | Daemon not running | Daemon auto-starts, check `probe lsp status` |
| **Empty responses** | Symbol not at function definition | Use exact function name position |
| **Incomplete message** | Concurrent request conflict | Retry the operation |

### Language Server Timings

- **rust-analyzer**: 10-15s initial indexing for large projects
- **pylsp**: 2-3s for Python projects
- **gopls**: 3-5s for Go modules
- **typescript-language-server**: 5-10s for node_modules

### Log Analysis Commands

```bash
# Check for errors
probe lsp logs -n 200 | grep ERROR

# Monitor specific language server
probe lsp logs --follow | grep rust-analyzer

# Check initialization timing
probe lsp logs | grep "initialize.*response"

# View call hierarchy requests
probe lsp logs | grep "prepareCallHierarchy\|incomingCalls\|outgoingCalls"
```

### Performance Monitoring

The in-memory log buffer stores:
- Timestamp with microsecond precision
- Log level (ERROR, WARN, INFO, DEBUG)
- Source file and line number
- Target component (e.g., "lsp_protocol", "lsp_stderr")
- Full message content including JSON-RPC payloads

### Daemon Communication Details

- Uses Unix domain sockets on macOS/Linux: `/var/folders/.../lsp-daemon.sock`
- Named pipes on Windows: `\\.\pipe\lsp-daemon`
- Binary protocol with MessagePack serialization
- UUID-based request tracking for concurrent operations

### Debugging Tips

1. **Check daemon logs**: `probe lsp logs -n 50`
2. **Monitor daemon status**: `probe lsp status`
3. **Test with netcat**: `echo -n '\x00\x00\x00\x04test' | nc -U /tmp/lsp-daemon.sock`
4. **Enable debug logging**: `LSP_LOG=1 probe lsp start -f`
5. **Check socket exists**: `ls -la /tmp/lsp-daemon.sock`

## Available Operations

The daemon supports these LSP operations:
- **Call Hierarchy**: Find all callers/callees of a function
- **Workspace Management**: Register multiple project roots
- **Server Status**: Monitor language server health
- **Log Access**: Retrieve daemon logs
- **Graceful Shutdown**: Clean termination with child cleanup

## Performance Considerations

- **Concurrent Clients**: Up to 100 simultaneous connections
- **Shared Servers**: One language server instance serves all clients
- **Response Time**: Most operations complete in <100ms
- **Memory Usage**: ~50MB base + language servers
- **CPU Usage**: Minimal when idle, spikes during indexing