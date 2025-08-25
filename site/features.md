# What is Probe?

Probe is an AI-friendly, fully local, semantic code search tool designed to power the next generation of AI coding assistants. It combines the speed of ripgrep with the understanding of tree-sitter to provide intelligent code search and extraction capabilities.

## Why Probe?

Most AI-assisted editors treat your code like plain text, splitting it into chunks that may or may not cover the full context—especially in large projects. **Probe** tackles this by combining ripgrep's speed with tree-sitter's AST parsing to return only the relevant blocks. No forced cloud indexing, no partial context.

This means:

- **Complete Context**: Get entire functions, classes, and code blocks—not just fragments.
- **Local Processing**: Probe runs entirely locally without requiring embedding generation. Your code stays on your machine during search operations.
- **AST Understanding**: Search based on code structure, not just text patterns.
- **Customizable Ranking**: Fine-tune how results are sorted using BM25, TF-IDF, or hybrid approaches.

## Core Capabilities

### Code Understanding

- **Complete Code Blocks**: Extracts entire functions, classes, or structs
- **AST Parsing**: Understands code structure using Abstract Syntax Trees
- **Semantic Search**: Finds code by concept and meaning
- **Language Awareness**: Recognizes language-specific patterns
- **Symbol Extraction**: Extracts code by function/class name

### Performance

- **Ripgrep Engine**: Fast initial text scanning
- **Optimized Parsing**: Efficient AST generation
- **Multi-Core Processing**: Parallel search operations
- **Session-Based Caching**: Avoids duplicate results

### Intelligent Ranking

- **TF-IDF**: Weighs terms based on frequency and importance
- **BM25**: Better relevance with document length normalization
- **Hybrid Ranking**: Multiple signals combined for superior ordering
- **Position Weighting**: Considers term position within code blocks

### Query Capabilities

- **Boolean Operators**: AND, OR, NOT for complex queries
- **Grouping**: Parentheses for logical term grouping
- **Term Modifiers**: +/- prefixes for required/excluded terms
- **Field Specifiers**: Target specific code elements
- **Smart Token Matching**: Stemming and compound word handling

### Privacy

- **Local Search Engine**: All search processing happens on your machine without requiring embedding generation
- **No Telemetry**: Zero data collection
- **Open Source**: Transparent codebase
- **AI Integration Note**: When using Probe with external AI services (Anthropic, OpenAI, etc.), code snippets are sent to those services as part of the AI interaction

### Language Support

- **Modern Languages**: Rust, Go, Python, JavaScript/TypeScript
- **Mainstream Languages**: Java, C++, C#, PHP
- **Documentation**: Markdown and other text formats
- **Language-Specific Parsing**: Tailored to each language's structure

## AI Integration

Probe offers powerful AI integration capabilities that allow you to leverage large language models to understand and navigate your codebase more effectively:

- **[AI Chat Mode](./ai-chat.md)**: Interactive CLI interface for asking questions about your codebase
- **[MCP Server Integration](./mcp-integration.md)**: Seamless integration with AI editors and assistants
- **[Node.js SDK](./nodejs-sdk.md)**: Programmatic access to Probe's code search capabilities

### AI-Specific Features

- **Token-Aware**: Limits results to fit AI context windows
- **Structured Output**: AI-friendly formats (JSON, XML)
- **Multi-Model Support**: Works with various LLM models
- **Streaming Responses**: Real-time AI interaction
- **OpenTelemetry Tracing**: Comprehensive monitoring and observability for AI interactions

## LSP Integration with Auto-Initialization

Probe integrates with Language Server Protocol (LSP) to provide IDE-level code intelligence with zero configuration:

- **[LSP Features Overview](./lsp-features.md)**: Call hierarchy, definitions, references, and hover information
- **[Indexing Documentation](./indexing-overview.md)**: Comprehensive guide to the LSP indexing system
- **Auto-Initialization**: Daemon starts automatically when using `--lsp` flag - no manual setup
- **Content-Addressed Caching**: 250,000x performance improvements with MD5-based cache invalidation
- **In-Memory Logging**: 1000 entries stored in memory, no disk I/O overhead
- **Multi-Language Support**: Rust, TypeScript, Python, Go, Java, and C/C++ with nested workspace discovery
- **Real-Time Analysis**: Background language servers with connection pooling for instant responses
- **Indexing System**: Project-wide indexing with progress tracking and language-specific pipelines

## Usage Modes

Probe can be used in multiple ways, depending on your workflow:

- **[CLI Mode](./cli-mode.md)**: Direct command-line interface
- **[MCP Server](./mcp-server.md)**: Integration with AI tools
- **[AI Chat](./ai-chat.md)**: Interactive AI assistant
- **[Web Interface](./web-interface.md)**: Browser-based exploration
- **[Node.js SDK](./nodejs-sdk.md)**: Programmatic access

## Advanced Features

### OpenTelemetry Tracing and Observability

Probe provides comprehensive monitoring and observability features for AI interactions:

- **Performance Monitoring**: Track AI response times, token usage, and throughput
- **File-based Tracing**: Save traces to JSON Lines format for offline analysis
- **Remote Tracing**: Send traces to OpenTelemetry collectors (Jaeger, Zipkin, etc.)
- **GitHub Actions Integration**: Automatic trace collection and artifact uploads
- **Rich Telemetry Data**: Capture model details, session information, and error tracking
- **Cost Optimization**: Monitor and analyze token consumption patterns

```bash
# Enable file-based tracing
probe-chat --trace-file ./traces.jsonl

# Enable remote tracing to Jaeger
probe-chat --trace-remote http://localhost:4318/v1/traces

# Enable console tracing for debugging
probe-chat --trace-console
```

### Pattern Matching

- **AST-Grep Patterns**: Search for specific code patterns using structural queries
- **Tree-Sitter Queries**: Language-aware structural search
- **Placeholder Variables**: Match function names, parameters, and bodies with variables
- **Structure-Aware Matching**: Finds code based on its structure, not just text content

### NPM Integration

- **Global Installation**: Simple `npm install -g @buger/probe@latest` for system-wide access
- **NPX Execution**: Run without installation via `npx -y @buger/probe-chat@latest`
- **Node.js API**: Programmatic access via JavaScript
- **Vercel AI SDK**: Streaming AI responses in web applications

## Getting Started

Ready to try Probe? Check out these resources:

- [Installation](./installation.md): Get Probe set up on your system
- [Quick Start](./quick-start.md): Learn the basics in minutes
- [How It Works](./how-it-works.md): Understand the technology behind Probe

## Use Cases

Probe is designed for a variety of workflows:

- **AI-Assisted Coding**: Provide your AI assistant with accurate code context
- **Code Exploration**: Quickly find and understand code in large repositories
- **Documentation Generation**: Extract code examples for documentation
- **Code Review**: Find patterns and anti-patterns across your codebase
- **Refactoring**: Identify all instances of a pattern that needs to be changed

For more detailed information on specific features, see:
- [How Probe Works](how-it-works.md)
- [Search Functionality](search-functionality.md)
- [Code Extraction](code-extraction.md)
