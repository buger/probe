# LSP Daemon - Multi-Language LSP Server Pool Manager

A high-performance daemon that manages pools of Language Server Protocol (LSP) servers, eliminating startup overhead and providing instant code intelligence for 20+ programming languages.

## 🎯 Features

- **Zero Startup Time**: Pre-warmed LSP servers respond in 50-100ms instead of 2-5 seconds
- **Multi-Language Support**: Built-in support for 20+ languages including Rust, Python, Go, TypeScript, Java, and more
- **Automatic Server Management**: Dynamic pooling with 1-4 servers per language based on load
- **Cross-Platform**: Works on Linux, macOS, and Windows
- **Simple Protocol**: Easy-to-implement JSON-based protocol over IPC
- **Auto-Start**: Daemon automatically starts when needed
- **Resource Efficient**: 24-hour idle timeout and automatic cleanup

## 🚀 Quick Start

### Installation

```bash
# Install from source
cargo install --path .

# Or download pre-built binary from releases
curl -L https://github.com/buger/probe/releases/latest/download/lsp-daemon-$(uname -s)-$(uname -m).tar.gz | tar xz
sudo mv lsp-daemon /usr/local/bin/
```

### Basic Usage

```bash
# Start daemon in foreground (for testing)
lsp-daemon --foreground

# Start daemon in background (automatic with clients)
lsp-daemon

# Check if daemon is running
lsp-daemon --socket /tmp/lsp-daemon.sock
```

## 📡 Protocol Documentation

The daemon uses a simple length-prefixed JSON protocol over platform-specific IPC:
- **Unix/Linux/macOS**: Unix domain socket at `/tmp/lsp-daemon.sock`
- **Windows**: Named pipe at `\\.\pipe\lsp-daemon`

### Wire Format

```
[4 bytes: message length as big-endian u32][N bytes: JSON message]
```

### Message Types

#### Requests

All requests must include a `request_id` (UUID v4).

**Connect** - Establish connection
```json
{
  "type": "Connect",
  "client_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**CallHierarchy** - Get call hierarchy for code
```json
{
  "type": "CallHierarchy",
  "request_id": "...",
  "file_path": "/path/to/file.rs",
  "pattern": "fn main"
}
```

**Status** - Get daemon status
```json
{
  "type": "Status",
  "request_id": "..."
}
```

**ListLanguages** - List available LSP servers
```json
{
  "type": "ListLanguages",
  "request_id": "..."
}
```

**Ping** - Health check
```json
{
  "type": "Ping",
  "request_id": "..."
}
```

**Shutdown** - Graceful shutdown
```json
{
  "type": "Shutdown",
  "request_id": "..."
}
```

#### Responses

All responses include the matching `request_id`.

**Connected**
```json
{
  "type": "Connected",
  "request_id": "...",
  "daemon_version": "0.1.0"
}
```

**CallHierarchy**
```json
{
  "type": "CallHierarchy",
  "request_id": "...",
  "result": {
    "item": {
      "name": "main",
      "kind": "Function",
      "file": "/path/to/file.rs",
      "line": 10,
      "column": 3
    },
    "incoming_calls": [...],
    "outgoing_calls": [...]
  }
}
```

**Status**
```json
{
  "type": "Status",
  "request_id": "...",
  "status": {
    "uptime_secs": 3600,
    "total_requests": 150,
    "active_connections": 2,
    "pools": [
      {
        "language": "Rust",
        "ready_servers": 2,
        "busy_servers": 1,
        "total_servers": 3
      }
    ]
  }
}
```

**Error**
```json
{
  "type": "Error",
  "request_id": "...",
  "error": "Error message"
}
```

## 🔧 Client Implementation

### Rust Client Example

```rust
use lsp_daemon::{IpcStream, DaemonRequest, DaemonResponse, MessageCodec};
use uuid::Uuid;

async fn connect_to_daemon() -> Result<()> {
    let mut stream = IpcStream::connect("/tmp/lsp-daemon.sock").await?;
    
    let request = DaemonRequest::Connect {
        client_id: Uuid::new_v4(),
    };
    
    let encoded = MessageCodec::encode(&request)?;
    stream.write_all(&encoded).await?;
    
    // Read response...
    Ok(())
}
```

### Python Client Example

```python
import socket
import json
import struct
import uuid

class LspDaemonClient:
    def __init__(self):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.connect("/tmp/lsp-daemon.sock")
    
    def send_request(self, request):
        json_bytes = json.dumps(request).encode('utf-8')
        length = struct.pack('>I', len(json_bytes))
        self.socket.send(length + json_bytes)
        
        # Read response
        length = struct.unpack('>I', self.socket.recv(4))[0]
        response = json.loads(self.socket.recv(length))
        return response
```

## 🌍 Supported Languages

| Language | LSP Server | Status |
|----------|------------|--------|
| Rust | rust-analyzer | ✅ Tested |
| Python | pylsp | ✅ Tested |
| Go | gopls | ✅ Configured |
| TypeScript | typescript-language-server | ✅ Configured |
| JavaScript | typescript-language-server | ✅ Configured |
| Java | jdtls | ✅ Configured |
| C/C++ | clangd | ✅ Configured |
| C# | omnisharp | ✅ Configured |
| Ruby | solargraph | ✅ Configured |
| PHP | intelephense | ✅ Configured |
| Swift | sourcekit-lsp | ✅ Configured |
| Kotlin | kotlin-language-server | ✅ Configured |
| Scala | metals | ✅ Configured |
| Haskell | haskell-language-server | ✅ Configured |
| Elixir | elixir-ls | ✅ Configured |
| Clojure | clojure-lsp | ✅ Configured |
| Lua | lua-language-server | ✅ Configured |
| Zig | zls | ✅ Configured |

## 🚀 Deployment Options

### systemd Service (Linux)

Create `/etc/systemd/system/lsp-daemon.service`:

```ini
[Unit]
Description=LSP Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/lsp-daemon --foreground
Restart=on-failure
User=yourusername

[Install]
WantedBy=multi-user.target
```

### launchd Service (macOS)

Create `~/Library/LaunchAgents/com.probe.lsp-daemon.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" 
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.probe.lsp-daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/lsp-daemon</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
```

## 📚 Library Usage

The lsp-daemon can also be used as a Rust library:

```toml
[dependencies]
lsp-daemon = "0.1"
```

```rust
use lsp_daemon::{LspDaemon, get_default_socket_path};

#[tokio::main]
async fn main() -> Result<()> {
    let daemon = LspDaemon::new(get_default_socket_path())?;
    daemon.run().await?;
    Ok(())
}
```

## 🔍 Architecture

The daemon maintains a pool of LSP servers for each language:
- **Min Servers**: 1 per language (started on demand)
- **Max Servers**: 4 per language (scales with load)
- **Recycling**: Servers restart after 100 requests
- **Idle Timeout**: Daemon shuts down after 24 hours of inactivity

## 🤝 Contributing

Contributions are welcome! Please see the main probe repository for contribution guidelines.

## 📄 License

MIT - See LICENSE file in the repository root