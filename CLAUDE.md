# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Probe is an AI-friendly, fully local, semantic code search tool built in Rust that combines ripgrep's speed with tree-sitter's AST parsing. It's designed to power AI coding assistants with precise, context-aware code search and extraction capabilities.

## Common Commands

### Building and Development
```bash
# Build the project (debug mode)
cargo build

# Build for release
cargo build --release

# Install locally
cargo install --path .

# Quick build using Makefile
make build
```

### Testing
```bash
# Run all tests
make test

# Run specific test types
make test-unit          # Unit tests only
make test-integration   # Integration tests only
make test-cli          # CLI tests only

# Run tests with backtrace
RUST_BACKTRACE=1 cargo test
```

### Code Quality
```bash
# Lint with clippy
make lint
# or
cargo clippy --all-targets --all-features -- -D warnings

# Format code
make format
# or
cargo fmt --all

# Fix all issues automatically
make fix-all
```

### Running the Application
```bash
# Run in debug mode
cargo run -- search "query" ./

# Run release build
cargo run --release -- search "query" ./

# Examples of common usage
probe search "function name" ./src
probe extract src/main.rs:42
probe search "error handling" --max-tokens 5000
```

### Testing Chat Example with Image Support
```bash
# First, set your API key (choose one):
export ANTHROPIC_API_KEY="your_key_here"
export OPENAI_API_KEY="your_key_here" 
export GOOGLE_API_KEY="your_key_here"

# Navigate to chat example
cd examples/chat

# Test chat with image URL (single message, non-interactive)
DEBUG_CHAT=1 GOOGLE_API_KEY=$GOOGLE_API_KEY MODEL_NAME=gemini-2.5-pro-preview-06-05 node index.js --force-provider google -m "Do you see whats on the image here https://github.com/user-attachments/assets/6c1292af-3e0b-4f45-8ef9-609102dea5fb"

# Run interactive chat mode
node index.js --force-provider google

# Run web interface (accessible at http://localhost:8080)
node index.js --web --port 8080
```

## Architecture Overview

### Core Components

**Language Support (`src/language/`)**
- Modular language parsers using tree-sitter grammars
- Supports Rust, JavaScript/TypeScript, Python, Go, C/C++, Java, Ruby, PHP, Swift, C#
- Each language implements the `Language` trait for AST parsing
- Factory pattern for language detection and instantiation

**Search Engine (`src/search/`)**
- Elastic query system with boolean operators (AND, OR, NOT)
- Multiple ranking algorithms: TF-IDF, BM25, and hybrid approaches
- Token-based search with stemming and stopword removal
- Result caching and optimization for large codebases

**Code Extraction (`src/extract/`)**
- AST-based code block extraction
- Symbol-aware extraction (functions, classes, structs)
- Context-preserving extraction with configurable limits

**CLI Interface (`src/cli.rs`)**
- Two main commands: `search` and `extract`
- Flexible output formats: markdown, JSON, plain text
- Token limiting for AI integration

### Key Files
- `src/main.rs` - Entry point and CLI argument parsing
- `src/lib.rs` - Library interface and main API
- `src/models.rs` - Core data structures and types
- `src/query.rs` - Query parsing and validation
- `src/ranking.rs` - Search result ranking algorithms

## Integration Points

### Examples Directory
- `examples/chat/` - AI chat interface using Anthropic/OpenAI APIs
- `mcp/` - Model Context Protocol server implementation
- `npm/` - Node.js SDK and CLI wrapper

### Multi-Platform Support
The project includes comprehensive cross-platform build configuration:
- GitHub Actions for automated releases
- Makefile targets for Linux, macOS (x86_64/ARM64), and Windows
- Platform-specific installation scripts

## Development Notes

### Adding New Languages
1. Add tree-sitter parser dependency to `Cargo.toml`
2. Create language module in `src/language/`
3. Implement the `Language` trait
4. Register in the language factory

### Performance Considerations
- Uses ripgrep for initial file scanning (extremely fast)
- AST parsing is parallelized using rayon
- Implements various caching mechanisms for repeated searches
- Token counting uses tiktoken-rs for AI integration

### Testing Strategy
- Comprehensive test suite covering parsing, search, and extraction
- Property-based testing with proptest
- CLI integration tests
- Performance benchmarks in `benches/`

### NPM Package Management
**IMPORTANT**: When adding new JavaScript files to `examples/chat/`, always update the `files` array in `examples/chat/package.json`. The npm package (@buger/probe-chat) is published from this directory and only includes files listed in the `files` array. Missing files will cause "Cannot find module" errors when the package is installed via npm.