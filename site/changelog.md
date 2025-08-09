---
title: Changelog
description: Release notes and changelog for Probe versions
layout: doc
---

# Changelog

All notable changes to Probe will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### 🚀 Major Features

#### Language Server Protocol (LSP) Integration
- **Full LSP support**: Advanced code intelligence through Language Server Protocol
- **Call hierarchy analysis**: See incoming and outgoing function calls
- **Multi-language support**: Rust, Python, Go, TypeScript, Java, C/C++ via language servers
- **High-performance daemon**: Background service managing language server pools
- **In-memory logging**: Circular buffer system storing last 1000 log entries
- **IPC communication**: Fast Unix sockets on macOS/Linux, named pipes on Windows
- **Server pooling**: Reuse warm servers for instant responses
- **Workspace awareness**: Maintains indexed state across requests
- **Real-time log following**: Monitor daemon activity with `--follow` flag
- **Automatic lifecycle management**: Servers spawn, initialize, and shutdown automatically
- **Concurrent request handling**: Multiple operations without blocking
- **Smart timeout handling**: Configurable timeouts for different operations

#### Cross-Platform Claude Code Backend Detection
- **Removed Unix-specific commands**: No more `which` command dependency, works on all platforms
- **Windows support**: Full support for `.cmd` files and Windows-specific paths
- **WSL integration**: Automatically detects Claude Code installed in WSL on Windows systems
- **Enhanced npm detection**: Better handling of npm global installations across different configurations
- **Improved error messages**: Clear guidance when Claude Code is not found

#### MCP Server Timeout Configuration
- **Configurable timeouts**: New `--timeout/-t` flag for both MCP servers
  - Basic MCP server: Default 30 seconds
  - Agent MCP server: Default 120 seconds
- **Per-request overrides**: Timeout can be specified in individual tool calls
- **Help command**: Added `--help/-h` flag to display usage information
- **Large codebase support**: Extended timeouts enable searching in massive repositories

#### Docker Integration & CI/CD
- **Official Docker images**: Pre-built images available on Docker Hub
  - `buger/probe` - CLI tool for code search
  - `buger/probe-chat` - AI-powered chat interface
- **Multi-platform support**: Images built for linux/amd64 and linux/arm64
- **Docker Compose configuration**: Easy local development setup
- **Automated CI/CD**: Docker images automatically built and published on releases
- **Security best practices**: Non-root execution, minimal base images, health checks
- **Production ready**: OCI-compliant labels, versioning, and comprehensive documentation

### 🔧 Improvements

#### Backend Detection Enhancements
- **Comprehensive logging**: Debug mode now shows detailed backend selection process
- **Multiple detection methods**: Tries direct execution, npm global, and common installation paths
- **Command caching**: Once found, Claude Code path is cached for faster subsequent calls
- **Graceful fallbacks**: Clear error messages when backends are unavailable

#### Documentation Updates
- **MCP timeout examples**: Updated READMEs with timeout configuration examples
- **Cross-platform guides**: Enhanced documentation for Windows and WSL users
- **Troubleshooting section**: Added common issues and solutions for backend detection

### 🐛 Bug Fixes
- Fixed Claude Code detection failing in GitHub Actions
- Fixed backend selection not respecting `implement_tool_backend` configuration
- Fixed Windows compatibility issues with spawn command

## [0.6.0] - 2025-07-17

### 🚀 Major Features

#### Implement Tool for Code Editing
- **New `implement` tool** for AI assistants to directly edit code files - see [CLI Mode documentation](./cli-mode.md#code-editing---allow-edit)
- Integrated with Aider for advanced code modification capabilities
- Configurable via `allow_edit` flag in [GitHub Actions workflows](./integrations/github-actions.md#code-modification-options)
- Enables AI assistants to make direct code changes during conversations

#### Enhanced GitHub Actions Integration
- **Allow Suggestions Feature**: New `allow_suggestions` flag for suggest-changes integration - see [GitHub Actions Integration](./integrations/github-actions.md#suggested-changes-allow_suggestions-true---recommended)
- **OpenTelemetry Tracing**: Full tracing support for monitoring AI model interactions - see [GitHub Actions Tracing](./integrations/github-actions.md#opentelemetry-tracing)
- **Failure Tagging**: Automatic tagging of failed GitHub Probe runs for better tracking
- **Improved Workflows**: Enhanced probe.yml with better error handling and configuration
- **Engineer Workflow**: New probe-engineer.yml for specialized engineering tasks - see [GitHub Actions examples](./integrations/github-actions.md#example-2-ai-engineer-code-editing-enabled)
- **Integration Testing**: Comprehensive integration test workflow added

#### OpenTelemetry Tracing Support
- **Complete Tracing Integration**: Full OpenTelemetry support for AI model monitoring - see [AI Chat Tracing](./ai-chat.md#opentelemetry-tracing)
- **File-based Tracing**: Save traces to JSON Lines format for offline analysis
- **Remote Tracing**: Send traces to OpenTelemetry collectors (Jaeger, Zipkin, etc.)
- **Performance Monitoring**: Track AI response times, token usage, and throughput
- **GitHub Actions Integration**: Automatic trace collection and artifact upload - see [GitHub Actions Tracing](./integrations/github-actions.md#opentelemetry-tracing)
- **Comprehensive Metrics**: Session tracking, error monitoring, and usage analytics
- **CLI Options**: New `--trace-file`, `--trace-remote`, and `--trace-console` flags

#### Crates.io Publishing
- **Automated Publishing**: Probe is now published to crates.io as a Rust library
- **Library Interface**: New `src/lib.rs` with public API for Rust integration
- **Release Automation**: Automatic crates.io publishing in release workflow

### 🔧 Improvements

#### AI Chat Enhancements
- **Chat History & Session Persistence**: Full session persistence with history dropdown navigation - see [Web Interface documentation](./web-interface.md#chat-history)
- **URL-based Session Routing**: Share and bookmark chat sessions with unique URLs (`/chat/session-id`)
- **Session Storage**: Automatic session storage to `~/.probe/sessions/` directory with JSON-based persistence
- **History Navigation**: Interactive history dropdown with session previews and metadata
- **Enhanced UI**: Always-visible header with clickable logo and improved navigation
- **Configurable Iterations**: `MAX_TOOL_ITERATIONS` environment variable support - see [AI Chat configuration](./ai-chat.md#configuration)
- **Enhanced Tool Support**: New file listing and search tools
- **Better Session Management**: Improved chat session handling and token tracking
- **Web Interface**: Enhanced web server with better error handling - see [Web Interface documentation](./web-interface.md)

#### MCP Protocol Updates
- **Mandatory Path Parameters**: Improved MCP tool definitions with required path parameters - see [MCP Server documentation](./mcp-server.md#available-tools)
- **Better Error Handling**: Enhanced error messages and validation
- **Tool Consistency**: Standardized tool interfaces across MCP implementations - see [MCP Integration guide](./mcp-integration.md)

#### Ripgrep Integration for Optimized Search Performance
- **Native Ripgrep Integration**: Integrated ripgrep Rust library for dramatically improved search performance
- **File Searching Performance**: 13% faster file searching (249ms → 217ms average improvement)
- **File I/O Performance**: 33% faster file I/O operations (303ms → 202ms average improvement)
- **AST Parsing Performance**: 32% faster tree parsing through optimized I/O operations
- **Memory Efficiency**: Enhanced memory mapping and streaming for large files with intelligent fallback
- **SIMD Optimizations**: Leveraged SIMD instructions for faster pattern matching where available
- **Pre-compiled Regex**: Eliminated redundant regex compilation by using pre-compiled RegexSet
- **Deterministic Results**: Ensured consistent search results across all runs
- **Fixed Timing Bug**: Corrected "Uncovered lines" measurement that was double-counting "term matching" time
- **RipgrepSearcher Module**: New dedicated searcher implementation with advanced encoding detection

#### Developer Experience
- **Windows Support**: Improved Windows compatibility for npm packages
- **Binary Management**: Enhanced binary download and path resolution
- **Documentation**: Updated documentation for new features and workflows

### 🐛 Bug Fixes

#### Search and File Handling
- **Underscore Directories**: Fixed recursive search in directories with underscores
- **Path Resolution**: Improved file path handling across different platforms
- **Binary Permissions**: Fixed Windows binary permission issues

#### GitHub Actions
- **Output Masking**: Fixed issue_number output to avoid GitHub Actions masking
- **Suggest-changes Integration**: Updated from reviewdog to suggest-changes action for better GitHub integration
- **Workflow Stability**: Multiple fixes for workflow reliability and error handling

#### Build and CI
- **Clippy Warnings**: Fixed uninlined_format_args warnings across codebase
- **Formatting**: Consistent code formatting with cargo fmt
- **Cross-platform**: Improved build compatibility across Linux, macOS, and Windows

### 📚 Documentation

#### Website Updates
- **Blog Infrastructure**: Added blog support with VitePress integration
- **Technical Guides**: New agentic flow guide with XML protocol documentation
- **Navigation**: Improved site navigation and structure
- **Discord Integration**: Updated Discord invite links

#### API Documentation
- **Tool Definitions**: Comprehensive documentation for all available tools
- **Configuration**: Detailed configuration options for GitHub Actions
- **Examples**: Enhanced examples and use case documentation

### 🔧 Infrastructure

#### Release Process
- **Multi-platform Builds**: Automated builds for Linux, macOS, Windows
- **NPM Publishing**: Automated npm package publishing
- **Version Management**: Improved version handling and release automation
- **Testing**: Enhanced integration and unit testing coverage

#### Development Tools
- **Linting**: Improved clippy and formatting rules
- **CI/CD**: Enhanced continuous integration with better error reporting
- **Dependencies**: Updated dependencies and security improvements

---

## [0.5.0] - Previous Release

For changes in version 0.5.0 and earlier, please refer to the [GitHub Releases](https://github.com/buger/probe/releases) page.

---

## Contributing

When contributing to Probe, please:

1. Follow the [Contributing Guidelines](https://github.com/buger/probe/blob/main/CONTRIBUTING.md)
2. Update this changelog for any user-facing changes
3. Use conventional commit messages for automatic changelog generation
4. Test your changes across supported platforms

## Links

- [GitHub Repository](https://github.com/buger/probe)
- [Release Downloads](https://github.com/buger/probe/releases)
- [NPM Package](https://www.npmjs.com/package/@buger/probe)
- [Crates.io Package](https://crates.io/crates/probe)
