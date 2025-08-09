---
title: LSP Features
description: Language Server Protocol integration for advanced code intelligence
---

# LSP Features

Probe integrates with Language Server Protocol (LSP) to provide IDE-level code intelligence from the command line.

## Overview

The LSP integration enables advanced code analysis features by leveraging the same language servers that power modern IDEs. This provides semantic understanding of code beyond simple text matching.

## Key Features

### Call Hierarchy Analysis

See the complete call graph for any function:

```bash
probe extract src/main.rs#calculate_result --lsp
```

Output includes:
- **Incoming Calls**: Functions that call this function
- **Outgoing Calls**: Functions that this function calls

Each call includes the exact file location for easy navigation.

### Multi-Language Support

Probe automatically detects and uses appropriate language servers:

- **Rust** - rust-analyzer
- **Python** - Python LSP Server (pylsp)
- **Go** - gopls
- **TypeScript/JavaScript** - typescript-language-server
- **Java** - Eclipse JDT Language Server
- **C/C++** - clangd

### High-Performance Architecture

The LSP daemon provides:
- Background server management
- Connection pooling for instant responses
- Workspace-aware caching
- Concurrent request handling

## Getting Started

### Basic Usage

Simply add the `--lsp` flag to extraction commands:

```bash
# Extract with LSP features
probe extract src/auth.rs#validate_user --lsp
```

The daemon starts automatically when needed.

### Daemon Management

```bash
# Check status
probe lsp status

# View logs
probe lsp logs

# Follow logs in real-time
probe lsp logs --follow

# Restart daemon
probe lsp restart
```

## Understanding Call Hierarchy

### Example Output

```
File: src/calculator.rs
Lines: 10-15
Type: function

LSP Information:
  Incoming Calls:
    - main (file:///src/main.rs:25)
    - test_calculate (file:///tests/calc_test.rs:10)
  Outgoing Calls:
    - add_numbers (file:///src/calculator.rs:20)
    - multiply (file:///src/calculator.rs:30)

fn calculate(a: i32, b: i32) -> i32 {
    let sum = add_numbers(a, b);
    multiply(sum, 2)
}
```

This shows:
- `calculate` is called by `main` and `test_calculate`
- `calculate` calls `add_numbers` and `multiply`

## Performance

### Initial Indexing

Language servers need time to analyze your codebase:
- Small projects: 1-3 seconds
- Medium projects: 5-10 seconds
- Large projects: 10-30 seconds

This only happens once - subsequent requests are instant.

### Optimization Tips

1. **Keep daemon running**: Better performance with warm servers
2. **Use release builds**: `cargo build --release` for production
3. **Pre-warm workspaces**: Run `probe lsp status` after opening projects

## Advanced Features

### Workspace Detection

The daemon automatically detects project roots by looking for:
- `Cargo.toml` (Rust)
- `package.json` (JavaScript/TypeScript)
- `go.mod` (Go)
- `pyproject.toml` or `setup.py` (Python)
- `pom.xml` or `build.gradle` (Java)

### Server Pooling

Multiple servers can run simultaneously:
- Different servers for different languages
- Multiple instances for concurrent requests
- Automatic cleanup of idle servers

### In-Memory Logging

Logs are stored in memory (last 1000 entries):
- No file permissions issues
- Zero disk I/O overhead
- Automatic rotation

## Troubleshooting

### No Call Hierarchy Data

**Cause**: Symbol not at function definition
**Solution**: Place cursor on function name, not inside body

### Slow Response

**Cause**: Language server indexing
**Solution**: Wait for initial indexing, then retry

### Connection Issues

**Cause**: Daemon not running
**Solution**: Run `probe lsp status` to auto-start

## Configuration

### Environment Variables

```bash
# Custom timeout (milliseconds)
PROBE_LSP_TIMEOUT=300000 probe extract file.rs#fn --lsp

# Custom socket path
PROBE_LSP_SOCKET=/custom/socket probe lsp start
```

### Debug Mode

```bash
# Start with debug logging
probe lsp start -f --log-level debug

# View debug logs
probe lsp logs -n 100
```

## Use Cases

### Code Review

Understand unfamiliar code quickly:
```bash
probe extract src/auth/handler.rs#authenticate --lsp
```

### Refactoring

Identify all callers before changing APIs:
```bash
probe extract src/api/v1.rs#deprecated_endpoint --lsp | grep "Incoming"
```

### Test Coverage

Find which tests exercise specific functions:
```bash
probe extract src/core.rs#critical_function --lsp | grep test_
```

### Documentation

Generate comprehensive function documentation:
```bash
probe extract src/lib.rs#public_api --lsp > docs/api.md
```

## Future Roadmap

Planned enhancements:
- Go-to definition
- Find all references
- Hover documentation
- Code completion
- Rename refactoring
- Quick fixes

## Learn More

- [Architecture Documentation](/docs/LSP_INTEGRATION.md)
- [Quick Reference](/docs/LSP_QUICK_REFERENCE.md)
- [Blog: LSP Integration Release](/blog/lsp-integration-release)