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

### High-Performance Persistent Cache Architecture

The LSP daemon provides a revolutionary three-layer cache system:

#### L1: Memory Cache (Ultra-Fast)
- **<1ms access time** for hot data in memory
- **LRU eviction** with configurable size limits
- **Concurrent access** with lock-free data structures

#### L2: Persistent Cache (Survives Restarts)
- **1-5ms access time** from disk-based sled database
- **Survives daemon restarts** and system reboots
- **MD5-based invalidation** ensures perfect cache accuracy
- **Compression support** to minimize disk usage
- **Content-addressed storage** with MD5-based cache keys

#### L3: LSP Servers (Computation Layer)
- **100ms-10s computation time** only on cache miss
- **Background server management** with persistent pools
- **Connection pooling** for instant responses
- **Auto-invalidation** when files change

#### Additional Features
- **In-memory logging** - 1000 entries, no disk I/O overhead
- **Concurrent request handling** - Multiple requests processed simultaneously
- **Cache import/export** - Team collaboration and sharing
- **Automatic cleanup** - Configurable TTL and size limits

## Getting Started

### Auto-Initialization

The LSP daemon automatically starts when you use the `--lsp` flag with any command:

```bash
# Extract with LSP features - daemon auto-starts if needed
probe extract src/auth.rs#validate_user --lsp

# Search with LSP enrichment - daemon auto-starts if needed
probe search "authentication" --lsp
```

**No manual setup required!** The daemon initialization is completely transparent:
- Automatically detects if daemon is running
- Starts daemon in background if needed  
- Waits for daemon to be ready
- Proceeds with your command

### Manual Daemon Management

While auto-initialization handles most use cases, you can also manage the daemon manually:

```bash
# Check daemon status and server pools
probe lsp status

# View in-memory logs (no files created)
probe lsp logs

# Follow logs in real-time
probe lsp logs --follow

# View more log entries
probe lsp logs -n 200

# Restart daemon (clears in-memory logs)
probe lsp restart

# Graceful shutdown
probe lsp shutdown

# Start in foreground with debug logging
probe lsp start -f --log-level debug
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

### Cache Performance Benefits

The content-addressed cache provides extraordinary performance improvements:

| Operation | First Call | Cached Call | Speedup |
|-----------|------------|-------------|----------|
| **Call Hierarchy** | 200-2000ms | 1-5ms | **250,000x+** |
| **Go to Definition** | 50-500ms | 1-3ms | **50,000x+** |
| **Find References** | 100-1000ms | 2-8ms | **100,000x+** |
| **Hover Information** | 30-200ms | 1-2ms | **30,000x+** |
| **Document Symbols** | 50-300ms | 1-2ms | **25,000x+** |
| **Workspace Symbols** | 100-1000ms | 5-10ms | **20,000x+** |

### Initial Indexing

Language servers need time to analyze your codebase:
- Small projects: 1-3 seconds
- Medium projects: 5-10 seconds  
- Large projects: 10-30 seconds

This only happens once - subsequent requests are instant thanks to caching.

### Optimization Tips

1. **Keep daemon running**: Better performance with warm servers and cache
2. **Use release builds**: `cargo build --release` for production
3. **Pre-warm workspaces**: Run `probe lsp init` after opening projects
4. **Monitor cache**: Use `probe lsp cache stats` to check hit rates
5. **Index in advance**: Use `probe lsp index --wait` for full project indexing

## Direct LSP Commands

Probe now offers direct access to all LSP operations through the `probe lsp call` command, providing IDE-level code intelligence from the command line.

### Available LSP Operations

#### Go to Definition

Find where a symbol is defined:

```bash
# Using line:column position
probe lsp call definition src/main.rs:42:10

# Using symbol name (automatically locates symbol)
probe lsp call definition src/main.rs#main_function
```

Output includes precise location information:
```
Definition for 'main_function':
  Location: /project/src/main.rs:42:10
  Symbol: main_function (function)
```

#### Find All References

Find all locations where a symbol is used:

```bash
# Find references without declaration
probe lsp call references src/auth.rs:25:8

# Include declaration in results
probe lsp call references src/auth.rs#validate_user --include-declaration
```

Output shows all usages:
```
References to 'validate_user':
  - /project/src/main.rs:15:5 (function call)
  - /project/src/handlers.rs:42:12 (function call)
  - /project/tests/auth_test.rs:10:8 (test usage)
```

#### Get Hover Information

Get documentation and type information:

```bash
# Get hover info at specific position
probe lsp call hover src/api.rs:18:5

# Get hover for specific symbol
probe lsp call hover src/types.rs#UserAccount
```

Output includes documentation:
```
Hover for 'UserAccount':
  Type: struct UserAccount
  Documentation:
    Represents a user account with authentication details.
    
    Fields:
    - id: String - Unique user identifier
    - email: String - User email address
    - created_at: DateTime<Utc> - Account creation time
```

#### Document Symbols

List all symbols in a file:

```bash
# Get all symbols in a file
probe lsp call document-symbols src/lib.rs
```

Output shows file structure:
```
Document symbols for 'src/lib.rs':
  - mod auth (module) at line 10
  - struct User (struct) at line 25
    - fn new (method) at line 30
    - fn validate (method) at line 45
  - fn main (function) at line 60
```

#### Workspace Symbols

Search for symbols across the entire workspace:

```bash
# Search for symbols containing "user"
probe lsp call workspace-symbols "user"

# Limit results
probe lsp call workspace-symbols "auth" --max-results 10
```

Output shows workspace-wide matches:
```
Workspace symbols matching 'user':
  - User (struct) in src/types.rs:25
  - UserService (struct) in src/services.rs:15  
  - authenticate_user (function) in src/auth.rs:42
  - create_user (function) in src/handlers.rs:18
```

#### Call Hierarchy (Advanced)

Get detailed call relationships:

```bash
# Get call hierarchy for a function
probe lsp call call-hierarchy src/calculator.rs#calculate
```

Output shows incoming and outgoing calls:
```
Call hierarchy for 'calculate':
  
  📥 Incoming calls (functions that call this):
     ← main (src/main.rs:25)
     ← test_calculate (tests/calc_test.rs:10)
  
  📤 Outgoing calls (this function calls):
     → add_numbers (src/calculator.rs:20)
     → multiply (src/calculator.rs:30)
```

#### Implementations

Find all implementations of an interface or trait:

```bash
# Find trait implementations
probe lsp call implementations src/traits.rs#Display

# Find interface implementations  
probe lsp call implementations src/interfaces.ts:15:8
```

#### Type Definition

Go to the type definition of a symbol:

```bash
# Find type definition
probe lsp call type-definition src/main.rs:42:10
```

### Location Syntax

Probe supports two location formats for maximum flexibility:

**Line:Column Format** (precise positioning):
```bash
probe lsp call definition file.rs:42:10
#                          ^   ^  ^
#                        file line column
```

**Symbol Name Format** (automatic location):
```bash
probe lsp call references file.rs#function_name
#                          ^     ^
#                        file   symbol
```

The symbol format automatically finds the symbol definition and uses its precise location.

### Performance Benefits

All direct LSP commands benefit from Probe's sophisticated caching:

- **First call**: 100ms-10s (LSP server computation)
- **Cached calls**: 1-5ms (nearly instant)
- **Cross-session**: Persistent cache survives daemon restarts

## Per-Workspace Cache Management

Probe implements sophisticated per-workspace caching that isolates cache entries by project:

### Workspace Cache Benefits

- **Isolation**: Each workspace has its own cache, preventing pollution
- **Monorepo Support**: Nested workspaces get separate caches automatically
- **Smart Routing**: Files are cached in the nearest workspace
- **Team Collaboration**: Workspace-specific caches can be shared
- **Resource Management**: LRU eviction of unused workspace caches

### Cache Directory Structure

```bash
# macOS: ~/Library/Caches/probe/lsp/workspaces/
# Linux: ~/.cache/probe/lsp/workspaces/
# Windows: %LOCALAPPDATA%/probe/lsp/workspaces/

├── abc123_my-rust-project/
│   ├── call_graph.db          # sled database
│   └── metadata.json          # cache statistics
├── def456_backend-service/
│   ├── call_graph.db
│   └── metadata.json
└── ghi789_frontend-app/
    ├── call_graph.db
    └── metadata.json
```

### Workspace Cache Commands

#### List Workspace Caches

```bash
# List all workspace caches
probe lsp cache list

# Detailed view with statistics
probe lsp cache list --detailed

# JSON output for scripting
probe lsp cache list --format json
```

Output shows all workspace caches:
```
Workspace Caches:
  abc123_my-rust-project (opened 2h ago, 1,234 entries, 45MB)
    Path: /Users/dev/projects/my-rust-project
    Last accessed: 5 minutes ago
    Hit rate: 94.2%
  
  def456_backend-service (opened 1d ago, 856 entries, 32MB)
    Path: /Users/dev/work/backend-service  
    Last accessed: 1 hour ago
    Hit rate: 89.1%
```

#### Get Workspace Cache Info

```bash
# Info for all workspaces
probe lsp cache info

# Info for specific workspace
probe lsp cache info --workspace /path/to/project

# JSON format
probe lsp cache info --format json
```

Detailed information includes:
- Cache size and entry count
- Hit/miss ratios
- Last access times
- Memory usage
- Workspace detection markers

#### Clear Workspace Caches

```bash
# Clear specific workspace cache
probe lsp cache clear-workspace --workspace /path/to/project

# Clear all workspace caches (with confirmation)
probe lsp cache clear-workspace

# Force clear without confirmation
probe lsp cache clear-workspace --force
```

### Workspace Detection

Probe automatically detects workspaces using these markers:

**Rust**: `Cargo.toml`
**TypeScript/JavaScript**: `package.json`, `tsconfig.json`
**Python**: `pyproject.toml`, `setup.py`, `requirements.txt`
**Go**: `go.mod`
**Java**: `pom.xml`, `build.gradle`
**C/C++**: `CMakeLists.txt`
**Generic**: `.git`, `README.md`

### Monorepo Support

For complex project structures with nested workspaces:

```bash
# Example monorepo structure
monorepo/
├── package.json          # Root workspace
├── backend/              
│   └── Cargo.toml        # Rust workspace  
├── frontend/
│   ├── package.json      # Frontend workspace
│   └── tsconfig.json
└── shared/
    └── utils.js          # Uses root workspace
```

Probe creates separate caches:
- `monorepo_root` for shared files and root-level code
- `backend_rust` for Rust backend code
- `frontend_ts` for TypeScript frontend code

Files are automatically routed to the nearest workspace cache.

### Configuration

```bash
# Configure workspace cache behavior
export PROBE_LSP_WORKSPACE_CACHE_MAX=8         # Max concurrent open caches
export PROBE_LSP_WORKSPACE_CACHE_SIZE_MB=100   # Size limit per workspace  
export PROBE_LSP_WORKSPACE_LOOKUP_DEPTH=3      # Max parent dirs to search

# Custom base cache directory
export PROBE_LSP_WORKSPACE_CACHE_DIR=/custom/cache/location
```

## Advanced Features

### Persistent Cache System

#### Content-Addressed Storage
Probe uses MD5 content hashing for intelligent cache invalidation:
- **Perfect invalidation** - MD5 content hashing detects any file changes
- **Content-based keys** - Same symbol in different file versions cached separately
- **Dependency tracking** - Related symbols invalidated together
- **Universal compatibility** - Works in CI, Docker, and non-git environments
- **Massive speedups** - 250,000x faster for repeated queries

#### Persistent Storage with sled
- **High-performance embedded database** for cache persistence
- **ACID transactions** ensure cache consistency
- **Compression** reduces disk usage by up to 70%
- **Multiple trees** for efficient indexing (nodes, files)
- **Automatic recovery** from corruption or version mismatches


```bash
# Configure persistent cache
export PROBE_LSP_PERSISTENCE_ENABLED=true
export PROBE_LSP_PERSISTENCE_PATH=~/.cache/probe/lsp/call_graph.db

# MD5-based cache management - works everywhere
probe lsp cache stats                         # Show cache statistics
probe lsp cache clear --file src/main.rs    # Clear specific file cache
probe lsp cache export project-cache.gz     # Export cache for sharing
```

```bash
# Comprehensive cache management
probe lsp cache stats                           # View cache performance and hit rates
probe lsp cache stats --detailed               # Include detailed cache information
probe lsp cache clear                          # Clear all caches (memory + persistent)
probe lsp cache clear --operation CallHierarchy # Clear specific operation type
probe lsp cache clear --file src/main.rs      # Clear cache for specific file
probe lsp cache clear --older-than 7          # Clear entries older than 7 days

# Cache import/export for team collaboration
probe lsp cache export project-cache.gz       # Export compressed cache
probe lsp cache export --operation CallHierarchy hierarchy-cache.gz
probe lsp cache import team-cache.gz          # Import shared cache

# Database maintenance
probe lsp cache compact                        # Optimize persistent storage
probe lsp cache cleanup                        # Remove expired entries
```

### Workspace Detection

The daemon automatically detects project roots by looking for:
- `Cargo.toml` (Rust)
- `package.json` (JavaScript/TypeScript)
- `go.mod` (Go)
- `pyproject.toml` or `setup.py` (Python)
- `pom.xml` or `build.gradle` (Java)
- **Nested workspace support** - Automatically discovers all nested workspaces

### Indexing System

Powerful project-wide indexing with progress tracking:

```bash
# Start indexing current workspace
probe lsp index

# Index specific languages
probe lsp index --languages rust,typescript

# Index recursively with custom workers
probe lsp index --recursive --max-workers 8

# Check indexing status with details
probe lsp index-status --detailed

# Follow indexing progress
probe lsp index-status --follow

# Stop ongoing indexing
probe lsp index-stop
```

### Server Pooling

Multiple servers can run simultaneously:
- Different servers for different languages
- Multiple instances for concurrent requests
- Automatic cleanup of idle servers
- Health monitoring and automatic restart

### In-Memory Logging

Logs are stored in memory (last 1000 entries):
- No file permissions issues
- Zero disk I/O overhead
- Automatic rotation
- Microsecond-precision timestamps

## Troubleshooting

### No Call Hierarchy Data

**Cause**: Symbol not at function definition or language server still indexing
**Solution**: 
1. Ensure you're using the exact function name position
2. Wait 10-15s for rust-analyzer to complete indexing
3. Check daemon logs: `probe lsp logs`

### Slow Response

**Cause**: Language server indexing or cold cache
**Solution**: 
1. Wait for initial indexing (10-15s for Rust projects)
2. Subsequent requests will be 250,000x faster due to caching
3. Use `probe lsp index --wait` for full project indexing

### Connection Issues

**Cause**: Daemon startup issues or build lock conflicts
**Solution**: 
1. Daemon auto-starts with `--lsp` flag, no manual intervention needed
2. **Important**: Avoid Rust build lock conflicts:
   ```bash
   # WRONG - causes hangs due to build locks:
   cargo run -- lsp start -f &
   cargo run -- lsp status  # <-- This hangs!
   
   # CORRECT - build first, then use binary:
   cargo build
   ./target/debug/probe lsp start -f &
   ./target/debug/probe lsp status
   ```
3. Check logs: `probe lsp logs --follow`

### Cache Issues

**Cause**: Stale cache entries or memory pressure
**Solution**:
1. Clear cache: `probe lsp cache clear`
2. Check cache stats: `probe lsp cache stats`
3. Restart daemon to reset: `probe lsp restart`

## Configuration

### Environment Variables

#### Basic Configuration
```bash
# Custom timeout (milliseconds)
PROBE_LSP_TIMEOUT=300000 probe extract file.rs#fn --lsp

# Custom socket path
PROBE_LSP_SOCKET=/custom/socket probe lsp start
```

#### Persistent Cache Configuration
```bash
# Enable persistent cache (default: disabled for compatibility)
export PROBE_LSP_PERSISTENCE_ENABLED=true

# Cache storage location
export PROBE_LSP_PERSISTENCE_PATH=~/.cache/probe/lsp/call_graph.db

# Cache behavior is now based on file content MD5 hashing
# No git dependency - works in all environments (CI, Docker, non-git dirs)

# Performance tuning
export PROBE_LSP_PERSISTENCE_BATCH_SIZE=50     # Batch write operations
export PROBE_LSP_PERSISTENCE_INTERVAL_MS=1000  # Write frequency
export PROBE_LSP_CACHE_TTL_DAYS=30            # Auto-cleanup threshold
export PROBE_LSP_CACHE_COMPRESS=true          # Enable compression

# Memory and storage limits
export PROBE_LSP_CACHE_SIZE_MB=512            # Memory cache limit
export PROBE_LSP_PERSISTENCE_SIZE_MB=2048     # Persistent storage limit
```

### Debug Mode

```bash
# Start with debug logging
probe lsp start -f --log-level debug

# View debug logs
probe lsp logs -n 100
```

## Use Cases

### Code Review with Direct LSP Commands

Understand unfamiliar code quickly:
```bash
# Get function documentation
probe lsp call hover src/auth/handler.rs#authenticate

# See all places this function is called
probe lsp call references src/auth/handler.rs#authenticate

# Understand what this function calls
probe lsp call call-hierarchy src/auth/handler.rs#authenticate
```

### Refactoring with References

Identify all callers before changing APIs:
```bash
# Find all references to a deprecated function
probe lsp call references src/api/v1.rs#deprecated_endpoint --include-declaration

# Find implementations of an interface you're changing
probe lsp call implementations src/traits.rs#AuthProvider
```

### Test Coverage Analysis

Find which tests exercise specific functions:
```bash
# Find all references to see test usage
probe lsp call references src/core.rs#critical_function | grep test_

# Get workspace symbols for test functions
probe lsp call workspace-symbols "test_critical" --max-results 20
```

### Documentation Generation

Generate comprehensive function documentation:
```bash
# Get hover information for all public APIs
probe lsp call document-symbols src/lib.rs | grep "(function)" | while read symbol; do
  probe lsp call hover "src/lib.rs#$symbol"
done > docs/api.md
```

### Legacy Code Understanding

```bash
# Start with a file and understand its structure
probe lsp call document-symbols legacy/complex.rs

# Pick a function and see what calls it
probe lsp call references legacy/complex.rs#mysterious_function

# Understand what the function does
probe lsp call hover legacy/complex.rs#mysterious_function

# See what it calls internally  
probe lsp call call-hierarchy legacy/complex.rs#mysterious_function
```

### Multi-Workspace Development

```bash
# List all your workspace caches
probe lsp cache list --detailed

# Work in backend
cd backend/
probe lsp call definition src/main.rs#process_request

# Switch to frontend (different workspace cache)
cd ../frontend/
probe lsp call references src/api.ts#processRequest

# Check cache stats across workspaces
probe lsp cache info --format json
```

## Future Roadmap

Planned enhancements:
- Code completion via LSP
- Rename refactoring across workspace
- Quick fixes and code actions
- Semantic highlighting
- Inlay hints
- Code lens integration

## Learn More

### Comprehensive Indexing Documentation

For detailed information about Probe's LSP indexing system:

- **[📖 Indexing Overview](./indexing-overview.md)** - What is indexing, benefits, and key concepts
- **🏗️ [Architecture Guide](./indexing-architecture.md)** - Deep dive into system internals and data flow
- **⚙️ [Configuration Reference](./indexing-configuration.md)** - Complete configuration options and environment variables
- **💻 [CLI Reference](./indexing-cli-reference.md)** - Detailed command documentation
- **🔧 [Language-Specific Guide](./indexing-languages.md)** - How each language is indexed and optimized
- **⚡ [Performance Guide](./indexing-performance.md)** - Optimization strategies and benchmarks
- **🔌 [API Reference](./indexing-api-reference.md)** - Integration guide for developers

### Additional Resources

- [Architecture Documentation](/docs/LSP_INTEGRATION.md)
- [Quick Reference](/docs/LSP_QUICK_REFERENCE.md)
- [Blog: LSP Integration Release](/blog/lsp-integration-release)