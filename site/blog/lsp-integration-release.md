---
title: "Introducing LSP Integration: Advanced Code Intelligence for Probe"
date: 2025-08-09
author: Probe Team
description: "Probe now features full Language Server Protocol integration, bringing advanced code intelligence capabilities including call hierarchy analysis, semantic understanding, and multi-language support through a high-performance daemon architecture."
tags: [lsp, features, performance, architecture]
---

# Introducing LSP Integration: Advanced Code Intelligence for Probe

We're excited to announce a major enhancement to Probe: **full Language Server Protocol (LSP) integration**. This powerful feature brings IDE-level code intelligence to Probe's command-line interface, enabling deeper code analysis and understanding across multiple programming languages.

## What is LSP?

The Language Server Protocol, originally developed by Microsoft for Visual Studio Code, provides a standard way for tools to communicate with language-specific servers that understand code semantics. This means Probe can now leverage the same powerful analysis engines that power modern IDEs.

## Key Features

### 🔍 Call Hierarchy Analysis

One of the most powerful features is call hierarchy analysis. When extracting code with Probe, you can now see:

- **Incoming Calls**: Which functions call the target function
- **Outgoing Calls**: Which functions the target calls

```bash
# Extract a function with full call hierarchy
probe extract src/main.rs#calculate_result --lsp

# Output includes:
# LSP Information:
#   Incoming Calls:
#     - main (file:///src/main.rs:10)
#     - test_calculate (file:///src/tests.rs:25)
#   Outgoing Calls:
#     - perform_calculation (file:///src/main.rs:50)
#     - apply_modifier (file:///src/main.rs:60)
```

This is invaluable for understanding code dependencies and impact analysis when refactoring.

### 🚀 High-Performance Daemon Architecture

We've implemented a sophisticated daemon architecture that:

- **Maintains server pools** for each language
- **Reuses warm servers** for instant responses
- **Handles concurrent requests** efficiently
- **Manages server lifecycle** automatically

The daemon runs in the background and manages all language servers, eliminating startup overhead and maintaining indexed code state across requests.

### 📊 In-Memory Logging System

Instead of writing logs to files (which can have permission issues), we've implemented an innovative in-memory circular buffer system:

- Stores last 1000 log entries in memory
- Zero file I/O overhead
- No permission issues
- Real-time log following with `--follow`

```bash
# View recent logs
probe lsp logs

# Follow logs in real-time
probe lsp logs --follow

# Get specific number of entries
probe lsp logs -n 100
```

### 🌍 Multi-Language Support

Currently supported languages include:

- **Rust** (rust-analyzer)
- **Python** (pylsp)
- **Go** (gopls)
- **TypeScript/JavaScript** (typescript-language-server)
- **Java** (jdtls)
- **C/C++** (clangd)

Each language server is automatically detected and managed by the daemon.

## Technical Deep Dive

### Architecture Overview

The LSP integration consists of several key components:

1. **LSP Daemon**: A persistent service managing language servers
2. **Server Manager**: Handles server pools and lifecycle
3. **IPC Communication**: Fast Unix sockets (macOS/Linux) or named pipes (Windows)
4. **Protocol Layer**: Strongly-typed request/response system

### Performance Optimizations

We've implemented several optimizations for production use:

- **Server pooling**: Reuse warm servers instead of spawning new ones
- **Workspace caching**: Maintain indexed state across requests
- **Lazy initialization**: Servers start only when needed
- **Circular buffer logging**: Bounded memory usage for logs

### Real-World Performance

In our benchmarks with a Rust project containing 400+ dependencies:

- First request: 10-15 seconds (includes indexing)
- Subsequent requests: < 1 second
- Memory usage: Stable at ~200MB per language server
- Concurrent requests: Handled without blocking

## Getting Started

### Basic Usage

```bash
# Start the daemon (happens automatically)
probe lsp start

# Extract code with LSP features
probe extract src/main.rs#my_function --lsp

# Check daemon status
probe lsp status

# View logs for debugging
probe lsp logs
```

### Advanced Features

```bash
# Start daemon in foreground for debugging
probe lsp start -f --log-level debug

# Follow logs in real-time
probe lsp logs --follow

# Restart daemon to clear state
probe lsp restart
```

## Implementation Highlights

### Call Hierarchy Resolution

The implementation correctly handles both incoming and outgoing calls by:

1. Sending `textDocument/prepareCallHierarchy` to identify the target
2. Requesting `callHierarchy/incomingCalls` for callers
3. Requesting `callHierarchy/outgoingCalls` for callees
4. Parsing and formatting results with file locations

### Robust Error Handling

- Automatic daemon startup if not running
- Graceful handling of server crashes
- Timeout protection for slow operations
- Clear error messages for debugging

### Memory-Safe Logging

The in-memory logging system uses:
- `Arc<Mutex<VecDeque<LogEntry>>>` for thread-safe access
- Circular buffer limiting entries to 1000
- Custom tracing layer capturing all events
- Zero file I/O for better performance

## Use Cases

### 1. Code Review and Understanding

When reviewing pull requests or understanding unfamiliar code:
```bash
# See what calls this function and what it calls
probe extract src/auth/login.rs#validate_user --lsp
```

### 2. Refactoring Impact Analysis

Before refactoring, understand dependencies:
```bash
# Check all callers before changing function signature
probe extract src/api/handler.rs#process_request --lsp
```

### 3. Test Coverage Analysis

Identify which tests call specific functions:
```bash
# Find test functions calling production code
probe extract src/core/engine.rs#execute --lsp | grep test_
```

### 4. Documentation Generation

Extract functions with full context for documentation:
```bash
# Generate comprehensive function documentation
probe extract src/lib.rs#public_api --lsp > docs/api.md
```

## Performance Comparison

| Operation | Without LSP | With LSP (cold) | With LSP (warm) |
|-----------|------------|-----------------|-----------------|
| Extract function | 50ms | 10-15s | 200ms |
| Show context | 100ms | 10-15s | 300ms |
| Multiple extracts | 500ms | 15s | 1s |

The initial indexing cost is amortized across multiple operations, making LSP integration highly efficient for sustained use.

## Future Roadmap

We're planning several enhancements:

- **Go-to Definition**: Navigate to symbol definitions
- **Find References**: Locate all usages of symbols
- **Hover Documentation**: Inline documentation display
- **Code Completion**: Suggestions for AI assistants
- **Rename Refactoring**: Safe symbol renaming
- **Code Actions**: Quick fixes and refactoring suggestions

## Technical Challenges Solved

### 1. Stdin Deadlock Prevention

We solved complex async I/O issues with language servers by:
- Proper mutex handling in async contexts
- Non-blocking message passing
- Timeout protection on all operations

### 2. Response Disambiguation

LSP servers can send both requests and responses with the same ID. We solved this by:
- Checking for `method` field presence
- Proper response type validation
- Handling server-initiated requests

### 3. Cross-Platform Compatibility

The implementation works seamlessly across platforms:
- Unix domain sockets on macOS/Linux
- Named pipes on Windows
- Platform-specific path handling

## Conclusion

The LSP integration represents a significant leap forward for Probe, bringing IDE-level code intelligence to the command line. Whether you're analyzing code dependencies, understanding unfamiliar codebases, or building AI-powered development tools, the LSP features provide the semantic understanding needed for advanced code analysis.

The feature is available now in the latest version. We encourage you to try it out and share your feedback!

## Try It Now

```bash
# Install or update Probe
cargo install probe-code

# Extract code with call hierarchy
probe extract your_file.rs#function_name --lsp

# Explore the daemon
probe lsp status
probe lsp logs --follow
```

Join our community and share your experiences with the new LSP integration!