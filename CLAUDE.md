# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with the probe codebase.

**IMPORTANT**: ALWAYS use code-search-agent to navigate the codebase and ask questions about it.

## Project Overview

Probe is an AI-friendly, fully local, semantic code search tool built in Rust that combines ripgrep's speed with tree-sitter's AST parsing. It's designed to power AI coding assistants with precise, context-aware code search and extraction capabilities.

## Development Guidelines

### 1. Testing Requirements

**EVERY feature, bug fix, or change MUST include tests:**
- Unit tests for new functions/modules (in same file using `#[cfg(test)]`)
- Integration tests for cross-module functionality (in `tests/` directory)
- CLI tests for command-line interface changes (`tests/cli_tests.rs`)
- Property-based tests with `proptest` for complex logic
- Test coverage for edge cases and error conditions

**Test patterns in this codebase:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_name() {
        // Arrange
        let input = create_test_data();
        
        // Act
        let result = function_under_test(input);
        
        // Assert
        assert_eq!(result, expected_value);
        assert!(condition);
        assert_ne!(unexpected, actual);
    }
}
```

**Before committing:**
```bash
make test           # Run all tests (unit + integration + CLI)
make test-unit      # Run unit tests only
make test-cli       # Run CLI tests
make test-property  # Run property-based tests
```

### 2. Error Handling

**Always use proper error handling with anyhow:**
```rust
use anyhow::{Context, Result};

// Good - use Result<T> with context
pub fn parse_file(path: &Path) -> Result<ParsedData> {
    let content = fs::read_to_string(path)
        .context(format!("Failed to read file: {:?}", path))?;
    
    parse_content(&content)
        .context("Failed to parse file content")
}

// Bad - using unwrap() in production code
pub fn parse_file(path: &Path) -> ParsedData {
    let content = fs::read_to_string(path).unwrap(); // NO!
    parse_content(&content).unwrap() // NO!
}
```

**Key patterns:**
- Return `Result<T>` for all fallible operations
- Use `.context()` to add error context
- Use `anyhow::Error` for flexible error handling
- Create custom error types when domain-specific errors needed
- Never use `.unwrap()` except in tests

### 3. Code Quality Standards

**Before EVERY commit run these commands:**
```bash
make format         # Format code with rustfmt
make lint           # Run clippy linter
make test           # Run all tests
# OR simply:
make fix-all        # Runs format + lint fixes + tests
```

**Code organization principles:**
- Keep modules focused and single-purpose
- Use descriptive names (avoid abbreviations like `cfg`, `ctx`)
- Add doc comments (`///`) for all public APIs
- Keep functions under 50 lines when possible
- Group related functionality in modules

### 4. Performance Considerations

- Profile before optimizing using `DEBUG=1` environment variable
- Document performance-critical code with comments
- Prefer lazy evaluation and iterators over collecting
- Use `rayon` for CPU-bound parallel tasks
- Leverage caching for expensive operations
- See `docs/PERFORMANCE_OPTIMIZATION.md` for detailed methodology

### 5. Adding New Features

**Process for new features:**
1. Search existing code for similar patterns using probe itself
2. Write tests FIRST (TDD approach)
3. Implement the feature
4. Ensure all tests pass
5. Update relevant documentation
6. Run `make fix-all` before committing

**For new language support:**
1. Add tree-sitter parser to `Cargo.toml`
2. Create module in `src/language/` implementing `Language` trait
3. Add comprehensive tests including test detection
4. Register in `src/language/factory.rs`
5. Update docs in `site/supported-languages.md`

## Common Commands

```bash
# Building
make build              # Debug build
cargo build --release   # Release build with optimizations

# Testing & Quality
make test              # All tests
make test-unit         # Unit tests only
make lint              # Run clippy
make format            # Format code
make fix-all           # Fix everything automatically

# Running probe
cargo run -- search "query" ./path
cargo run -- extract file.rs:42
probe search "function" ./src --max-results 10

# Performance debugging
DEBUG=1 probe search "query" ./path  # Shows timing information
```

## Architecture Quick Reference

### Core Components
- `src/language/` - Language parsers using tree-sitter
- `src/search/` - Search engine with Elastic query support
- `src/extract/` - Code extraction logic
- `src/ranking.rs` - BM25 and TF-IDF ranking
- `src/simd_ranking.rs` - SIMD-optimized ranking

### Key Design Patterns
- **Result<T> everywhere**: All fallible operations return Result
- **Lazy evaluation**: Parse only what's needed
- **Parallel processing**: Use rayon for file processing
- **Caching**: Multiple cache levels (parser pool, tree cache, search cache)
- **Zero-copy where possible**: Use references and slices

## GitHub Integration

**ALWAYS use `gh` CLI for GitHub operations:**
```bash
gh issue view <number> --comments    # View issue with full discussion
gh pr view <number> --comments       # View PR with reviews
gh pr create                         # Create pull request
```

## Project-Specific Patterns

### Testing Patterns
- Unit tests use `#[cfg(test)]` modules in same file
- Integration tests go in `tests/` directory  
- Use `assert!`, `assert_eq!`, `assert_ne!` macros
- Property tests with `proptest` for fuzzing
- Test both success and error cases

### Module Organization
```rust
// Standard module layout
pub mod module_name;              // Public module
mod internal_module;              // Private module
pub use module_name::PublicItem;  // Re-exports

#[cfg(test)]
mod tests {                       // Test module
    use super::*;
}
```

### Common Idioms
- Use `Arc<>` for shared immutable data
- Prefer `&str` over `String` for function parameters
- Use `PathBuf` for owned paths, `&Path` for borrowed
- Implement `Display` and `Debug` for public types
- Use `#[derive()]` for common traits

## Important Notes

### NPM Package Management
When adding files to `examples/chat/`, ALWAYS update the `files` array in `examples/chat/package.json`.

### Pre-commit Hook
The project has git hooks in `.githooks/`. Install with:
```bash
make install-hooks
```

### Debugging Tips
- Use `DEBUG=1` for verbose output
- Check `error.log` for detailed errors
- Use `RUST_BACKTRACE=1` for stack traces
- Profile with `cargo flamegraph` for performance

### LSP Architecture & Debugging

#### Architecture Overview
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

#### Debugging LSP Issues

**CRITICAL: Avoid Rust Build Lock Contention**
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

**1. View LSP daemon logs (in-memory, no files):**
```bash
probe lsp logs              # View last 50 log entries
probe lsp logs -n 100       # View last 100 entries
probe lsp logs --follow     # Follow logs in real-time (polls every 500ms)
```

**2. Check daemon status and server pools:**
```bash
probe lsp status            # Show daemon status, uptime, and server pools
probe lsp shutdown          # Stop daemon cleanly
probe lsp restart           # Restart daemon (clears in-memory logs)
```

**3. Debug in foreground mode:**
```bash
# Run daemon in foreground with debug logging
./target/debug/probe lsp start -f --log-level debug

# In another terminal, test LSP operations
./target/debug/probe extract file.rs#symbol --lsp
```

**4. Common LSP issues and solutions:**

| Issue | Cause | Solution |
|-------|-------|----------|
| **No call hierarchy data** | Language server still indexing | Wait 10-15s for rust-analyzer to index |
| **Timeout errors** | Large codebase or slow language server | Increase timeout in client config |
| **Connection refused** | Daemon not running | Daemon auto-starts, check `probe lsp status` |
| **Empty responses** | Symbol not at function definition | Use exact function name position |
| **Incomplete message** | Concurrent request conflict | Retry the operation |

**5. Language Server Timings:**
- **rust-analyzer**: 10-15s initial indexing for large projects
- **pylsp**: 2-3s for Python projects
- **gopls**: 3-5s for Go modules
- **typescript-language-server**: 5-10s for node_modules

**6. Log Analysis Commands:**
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

**7. Performance Monitoring:**
The in-memory log buffer stores:
- Timestamp with microsecond precision
- Log level (ERROR, WARN, INFO, DEBUG)
- Source file and line number
- Target component (e.g., "lsp_protocol", "lsp_stderr")
- Full message content including JSON-RPC payloads

**8. Daemon Communication:**
- Uses Unix domain sockets on macOS/Linux: `/var/folders/.../lsp-daemon.sock`
- Named pipes on Windows: `\\.\pipe\lsp-daemon`
- Binary protocol with JSON serialization
- UUID-based request tracking for concurrent operations
- See `docs/LSP_CLIENT_GUIDE.md` for complete client implementation guide

### Per-Workspace Cache System

#### What is Per-Workspace Caching?

Probe now implements sophisticated per-workspace caching that creates separate cache instances for each workspace, enabling:

**Key Benefits:**
- **Isolation**: Each project has its own cache, preventing cache pollution between projects
- **Monorepo Support**: Nested workspaces in monorepos get their own caches automatically
- **Intelligent Routing**: Files are cached in the nearest workspace (e.g., backend/src/main.rs goes to backend workspace)
- **Team Collaboration**: Workspace-specific caches can be shared within teams
- **Resource Management**: LRU eviction of least-used workspace caches when memory limits are reached

#### Cache Directory Structure

```
~/Library/Caches/probe/lsp/workspaces/         # macOS
~/.cache/probe/lsp/workspaces/                  # Linux
%LOCALAPPDATA%/probe/lsp/workspaces/            # Windows

├── abc123_my-rust-project/
│   ├── call_graph.db                          # sled database
│   └── metadata.json                          # cache statistics
├── def456_backend-service/
│   ├── call_graph.db
│   └── metadata.json
└── ghi789_frontend-app/
    ├── call_graph.db
    └── metadata.json
```

**Directory Naming Convention:**
- Format: `{workspace_hash}_{workspace_name}/`
- Hash: First 6 chars of SHA256 hash of workspace absolute path
- Name: Sanitized workspace directory name (safe for filesystems)

#### Cache Resolution Strategy

The system uses a **nearest workspace wins** strategy:

1. **File Analysis**: For any file (e.g., `/project/backend/src/auth.rs`)
2. **Workspace Discovery**: Walk up directory tree looking for workspace markers
3. **Workspace Selection**: Choose nearest workspace (`/project/backend/` beats `/project/`)
4. **Cache Routing**: Route all cache operations to that workspace's cache

**Workspace Detection Markers:**
- **Rust**: `Cargo.toml`
- **TypeScript/JavaScript**: `package.json`, `tsconfig.json`
- **Python**: `pyproject.toml`, `setup.py`, `requirements.txt`
- **Go**: `go.mod`
- **Java**: `pom.xml`, `build.gradle`
- **C/C++**: `CMakeLists.txt`
- **Generic**: `.git`, `README.md`

#### CLI Commands for Workspace Cache Management

**List workspace caches:**
```bash
probe lsp cache list                           # Show all workspace caches
probe lsp cache list --detailed               # Include cache statistics
probe lsp cache list --format json            # JSON output for scripting
```

**View workspace cache information:**
```bash
probe lsp cache info                           # Show info for all workspaces
probe lsp cache info /path/to/workspace        # Show info for specific workspace
probe lsp cache info --format json            # JSON format
```

**Clear workspace caches:**
```bash
probe lsp cache clear-workspace                # Clear all workspace caches (with confirmation)
probe lsp cache clear-workspace /path/to/workspace  # Clear specific workspace
probe lsp cache clear-workspace --force        # Skip confirmation prompt
```

**Cache statistics:**
```bash
probe lsp cache stats                          # Combined stats across all workspaces
probe lsp cache stats --detailed              # Per-workspace breakdown
```

#### Configuration

**Environment Variables:**
```bash
# Configure workspace cache behavior
export PROBE_LSP_WORKSPACE_CACHE_MAX=8         # Max concurrent open caches (default: 8)
export PROBE_LSP_WORKSPACE_CACHE_SIZE_MB=100   # Size limit per workspace (default: 100MB)
export PROBE_LSP_WORKSPACE_LOOKUP_DEPTH=3      # Max parent dirs to search (default: 3)

# Base cache directory (if you want to change the location)
export PROBE_LSP_WORKSPACE_CACHE_DIR=/custom/path/to/caches
```

**Configuration File (Optional):**
```toml
# ~/.config/probe/lsp.toml
[workspace_cache]
max_open_caches = 8
cache_size_mb_per_workspace = 100
max_parent_lookup_depth = 3
base_cache_dir = "~/Library/Caches/probe/lsp/workspaces"

[workspace_cache.ttl]
days = 30                    # Clean up entries older than 30 days
compress = true              # Enable compression for storage
```

#### Troubleshooting Workspace Cache Issues

**1. Cache Directory Permissions:**
```bash
# Check cache directory exists and is writable
ls -la ~/Library/Caches/probe/lsp/workspaces/
# Should show drwx------ (700) permissions

# Fix permissions if needed
chmod 700 ~/Library/Caches/probe/lsp/workspaces/
```

**2. Cache Not Found for File:**
```bash
# Debug workspace resolution for a specific file
probe lsp debug workspace /path/to/file.rs

# Check which workspace a file maps to
probe lsp cache info /path/to/project/
```

**3. Cache Performance Issues:**
```bash
# Check if too many caches are open
probe lsp cache stats --detailed

# Look for cache evictions in logs
probe lsp logs -n 100 | grep "evicted\|LRU"

# Increase max open caches if needed
export PROBE_LSP_WORKSPACE_CACHE_MAX=16
```

**4. Disk Space Issues:**
```bash
# Check cache sizes
probe lsp cache list --detailed

# Clean up old entries
probe lsp cache compact --clean-expired

# Clear unused workspace caches
probe lsp cache clear-workspace --force
```

#### Performance Implications

**Memory Usage:**
- Each open workspace cache uses ~5-20MB of RAM
- Default limit of 8 concurrent caches = ~40-160MB max
- LRU eviction automatically manages memory pressure

**Disk Usage:**
- Each workspace cache limited to 100MB by default
- Compressed storage reduces disk usage by ~60-70%
- Automatic cleanup of entries older than 30 days

**Cache Hit Rates:**
- Per-workspace caches typically achieve 90-95% hit rates
- Better isolation means fewer false cache misses
- Nested workspaces benefit from focused caching

#### Migration from Global Cache

**Automatic Migration:**
- No manual migration needed
- Old global cache continues to work as fallback
- New workspace caches gradually populate with usage
- Old cache can be cleared after workspace caches are established

**Verifying Migration:**
```bash
# Check that workspace caches are being used
probe lsp cache stats --detailed

# Should show multiple workspace entries, not just global cache
# Look for entries like "workspace_abc123_my-project"
```

#### Best Practices

**For Monorepos:**
- Each sub-project gets its own cache automatically
- Shared libraries cached in root workspace
- Configure larger cache limits for monorepos: `export PROBE_LSP_WORKSPACE_CACHE_MAX=16`

**For Development Teams:**
- Workspace caches can be backed up and shared
- Export/import commands work on per-workspace basis
- Cache names include workspace path hash for uniqueness

**For CI/CD:**
- Workspace caches work great in containerized environments
- No git dependencies - pure filesystem-based detection
- Cache sharing between builds of same workspace is automatic

## Getting Help

1. Search codebase first: `probe search "topic" ./src`
2. Check existing tests for usage examples
3. Review similar implementations
4. Consult docs in `site/` directory

Remember: **Quality > Speed**. Write tests, handle errors properly, and maintain code standards.

## LSP Client Implementation

For detailed information on implementing an LSP client that communicates with the probe daemon, see:
**[docs/LSP_CLIENT_GUIDE.md](docs/LSP_CLIENT_GUIDE.md)**

This guide includes:
- Complete client implementation examples (Python, Rust, TypeScript)
- Wire protocol specification
- Request/response types
- Socket path discovery
- Connection management best practices
- Debugging tips and common issues# Trigger CI re-run
# Triggering CI re-run
