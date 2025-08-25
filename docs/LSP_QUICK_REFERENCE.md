# LSP Quick Reference Guide

## Essential Commands

### Basic Usage
```bash
# Extract with call hierarchy
probe extract src/main.rs#function_name --lsp

# Start daemon manually (usually auto-starts)
probe lsp start

# Check status
probe lsp status
```

### Daemon Management
```bash
probe lsp start          # Start in background
probe lsp start -f       # Start in foreground (debug)
probe lsp status         # Check daemon and servers
probe lsp restart        # Restart daemon
probe lsp shutdown       # Stop daemon
```

### Log Viewing
```bash
probe lsp logs           # Last 50 entries
probe lsp logs -n 100    # Last 100 entries
probe lsp logs --follow  # Real-time following
```

## Supported Languages

| Language | Server | Auto-detected |
|----------|--------|---------------|
| Rust | rust-analyzer | ✓ |
| Python | pylsp | ✓ |
| Go | gopls | ✓ |
| TypeScript/JS | typescript-language-server | ✓ |
| Java | jdtls | ✓ |
| C/C++ | clangd | ✓ |

## Common Issues

### Slow First Request
**Problem**: First extraction takes 10-15 seconds
**Solution**: Normal - language server is indexing. Subsequent requests are fast.

### No Call Hierarchy
**Problem**: No incoming/outgoing calls shown
**Solution**: Ensure cursor is on function name, not inside function body.

### Build Lock Conflicts
**Problem**: `cargo run` commands hang
**Solution**: Build first, then use binary:
```bash
cargo build
./target/debug/probe lsp status
```

## Performance Tips

1. **Keep daemon running** - Start on system boot for best performance
2. **Pre-warm workspaces** - Run `probe lsp status` after opening project
3. **Use release builds** - `cargo build --release` for production
4. **Monitor logs** - `probe lsp logs --follow` when debugging

## Architecture at a Glance

```
probe extract --lsp
    ↓
LSP Client (IPC)
    ↓
LSP Daemon
    ↓
Server Manager
    ↓
Language Servers (rust-analyzer, pylsp, etc.)
```

## Log Levels

- **ERROR**: Critical failures
- **WARN**: Important warnings
- **INFO**: Normal operations
- **DEBUG**: Detailed debugging info

Set with: `probe lsp start -f --log-level debug`

## Advanced Usage

### Custom Socket Path
```bash
PROBE_LSP_SOCKET=/custom/path probe lsp start
```

### Extended Timeout
```bash
PROBE_LSP_TIMEOUT=300000 probe extract file.rs#fn --lsp
```

### Debug Protocol Messages
```bash
probe lsp logs | grep ">>> TO LSP\|<<< FROM LSP"
```

## Quick Debugging

```bash
# Is daemon running?
probe lsp status

# What's happening?
probe lsp logs --follow

# Restart everything
probe lsp restart

# Check specific server
probe lsp logs | grep rust-analyzer
```