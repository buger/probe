# LSP Integration Documentation

## Overview

Probe includes a powerful Language Server Protocol (LSP) integration that provides advanced code intelligence features. The LSP daemon manages multiple language servers efficiently, enabling features like call hierarchy analysis, code navigation, and semantic understanding across different programming languages.

## Architecture

### Components

```
┌─────────────────┐
│   CLI Client    │
│  (probe extract)│
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  LSP Client     │
│  (IPC Socket)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   LSP Daemon    │
│  (Server Pool)  │
└────────┬────────┘
         │
    ┌────┴────┬──────────┐
    ▼         ▼          ▼
┌────────┐┌────────┐┌────────┐
│ rust-  ││  pyls  ││ gopls  │
│analyzer│└────────┘└────────┘
└────────┘
```

### Key Components

1. **LSP Daemon**: A persistent background service that manages language server instances
   - Maintains server pools for each language
   - Handles concurrent requests efficiently
   - Manages server lifecycle (spawn, initialize, shutdown)
   - Implements in-memory circular buffer for logging

2. **Server Manager**: Manages pools of language servers
   - Creates servers on-demand
   - Reuses idle servers for performance
   - Handles server crashes and restarts
   - Workspace-aware server allocation

3. **LSP Client**: Communicates with the daemon via IPC
   - Unix domain sockets on macOS/Linux
   - Named pipes on Windows
   - Automatic daemon startup if not running
   - Request/response protocol with UUID tracking

4. **Protocol Layer**: Defines communication between client and daemon
   - Strongly-typed request/response messages
   - Support for various LSP operations
   - Efficient binary serialization

## Features

### Call Hierarchy

Analyze function/method relationships in your code:

```bash
# Extract with call hierarchy
probe extract src/main.rs#my_function --lsp

# Output includes:
# - Incoming calls (who calls this function)
# - Outgoing calls (what this function calls)
```

### Supported Languages

Currently supported language servers:
- **Rust**: rust-analyzer
- **Python**: pylsp (Python LSP Server)
- **Go**: gopls
- **TypeScript/JavaScript**: typescript-language-server
- **Java**: jdtls
- **C/C++**: clangd

### Daemon Management

```bash
# Start daemon in foreground (for debugging)
probe lsp start -f

# Start daemon in background
probe lsp start

# Check daemon status
probe lsp status

# View daemon logs
probe lsp logs           # Last 50 entries
probe lsp logs -n 100    # Last 100 entries
probe lsp logs --follow  # Real-time log following

# Restart daemon
probe lsp restart

# Shutdown daemon
probe lsp shutdown
```

## Configuration

### Environment Variables

- `PROBE_LSP_TIMEOUT`: Request timeout in milliseconds (default: 240000ms / 4 minutes)
- `PROBE_LSP_SOCKET`: Custom socket path for daemon communication

### Language Server Configuration

Language servers are automatically detected if installed in PATH. To use custom installations:

1. Ensure the language server binary is in your PATH
2. Or specify full path in language server configuration (future feature)

## Performance Considerations

### Server Pool Management

The daemon maintains a pool of language servers for each language:
- Idle servers are reused for new requests
- Servers are kept warm for frequently accessed workspaces
- Automatic cleanup of unused servers after timeout

### Memory Management

- In-memory log buffer limited to 1000 entries
- Circular buffer prevents unbounded memory growth
- Language servers are shared across requests when possible

### Indexing Time

Some language servers (especially rust-analyzer) require significant indexing time:
- First request to a workspace may take 10-30 seconds
- Subsequent requests are much faster (< 1 second)
- The daemon maintains indexed state across requests

## Troubleshooting

### Common Issues

1. **Daemon not starting**
   - Check if another instance is running: `ps aux | grep probe`
   - Remove stale socket file: `rm /tmp/lsp-daemon.sock`
   - Check permissions on socket directory

2. **Slow response times**
   - Language server is indexing (check logs)
   - Large workspace requires more time
   - Consider pre-warming with `probe lsp status`

3. **Missing call hierarchy data**
   - Ensure language server supports call hierarchy
   - Symbol might not be at a function definition
   - Try using the function name directly

4. **Connection errors**
   - Daemon may have crashed (check logs)
   - Socket permissions issue
   - Firewall blocking local connections (Windows)

### Debug Commands

```bash
# Enable debug logging
probe lsp start -f --log-level debug

# Check which servers are running
probe lsp status

# View detailed logs
probe lsp logs -n 200

# Test specific language server
probe extract test.rs#main --lsp --debug
```

### Log Analysis

The daemon logs provide detailed information:
- LSP protocol messages (requests/responses)
- Server lifecycle events (spawn, initialize, shutdown)
- Error messages from language servers
- Performance timing information

Example log analysis:
```bash
# Check for errors
probe lsp logs | grep ERROR

# Monitor specific language server
probe lsp logs --follow | grep rust-analyzer

# Check initialization time
probe lsp logs | grep "initialize.*response"
```

## Best Practices

1. **Start daemon on system startup** for better performance
2. **Pre-warm frequently used workspaces** with a status check
3. **Monitor logs** when debugging integration issues
4. **Use release builds** for production (`cargo build --release`)
5. **Restart daemon** after major code changes to clear caches

## API Reference

### Client Methods

- `get_status()`: Get daemon status and server information
- `get_call_hierarchy()`: Retrieve call hierarchy for a symbol
- `list_languages()`: List supported language servers
- `get_logs(lines)`: Retrieve recent log entries
- `shutdown()`: Gracefully shutdown the daemon

### Protocol Types

- `DaemonRequest`: Client-to-daemon requests
- `DaemonResponse`: Daemon-to-client responses
- `CallHierarchyInfo`: Incoming/outgoing call information
- `LogEntry`: Structured log entry with timestamp and level

## Future Enhancements

- [ ] Streaming log support for real-time monitoring
- [ ] Custom language server configurations
- [ ] Multi-root workspace support
- [ ] Semantic token highlighting
- [ ] Go-to definition/references
- [ ] Hover documentation
- [ ] Code completion suggestions
- [ ] Rename refactoring
- [ ] Code actions and quick fixes