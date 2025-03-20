<p align="center">
  <img src="logo.png?2" alt="Probe Logo" width="400">
</p>

# Probe

Probe is an **AI-friendly, fully local, semantic code search** tool designed to power the next generation of AI coding assistants. By combining the speed of [ripgrep](https://github.com/BurntSushi/ripgrep) with the code-aware parsing of [tree-sitter](https://tree-sitter.github.io/tree-sitter/), Probe delivers precise results with complete code blocks—perfect for large codebases and AI-driven development workflows.

---

## Table of Contents

- [Quick Start](#quick-start)
- [Features](#features)
- [Installation](#installation)
  - [Quick Installation](#quick-installation)
  - [Requirements](#requirements)
  - [Manual Installation](#manual-installation)
  - [Building from Source](#building-from-source)
  - [Verifying the Installation](#verifying-the-installation)
  - [Troubleshooting](#troubleshooting)
  - [Uninstalling](#uninstalling)
- [Usage](#usage)
  - [CLI Mode](#cli-mode)
  - [MCP Server Mode](#mcp-server-mode)
  - [AI Chat Mode](#ai-chat-mode) (Example in examples/chat)
  - [Web Interface](#web-interface)
- [Supported Languages](#supported-languages)
- [How It Works](#how-it-works)
- [Adding Support for New Languages](#adding-support-for-new-languages)
- [Releasing New Versions](#releasing-new-versions)

---

## Quick Start

**NPM Installation**
The easiest way to install Probe is via npm, which also installs the binary:

~~~bash
npm install -g @buger/probe
~~~

**Basic Search Example**
Search for code containing the phrase "llm pricing" in the current directory:

~~~bash
probe search "llm pricing" ./
~~~

**Advanced Search (with Token Limiting)**
Search for "partial prompt injection" in the current directory but limit the total tokens to 10000 (useful for AI tools with context window constraints):

~~~bash
probe search "prompt injection" ./ --max-tokens 10000
~~~

**Elastic Search Queries**
Use advanced query syntax for more powerful searches:

~~~bash
# Use AND operator for terms that must appear together
probe search "error AND handling" ./

# Use OR operator for alternative terms
probe search "login OR authentication OR auth" ./src

# Group terms with parentheses for complex queries
probe search "(error OR exception) AND (handle OR process)" ./

# Use wildcards for partial matching
probe search "auth* connect*" ./

# Exclude terms with NOT operator
probe search "database NOT sqlite" ./
~~~

**Extract Code Blocks**
Extract a specific function or code block containing line 42 in main.rs:

~~~bash
probe extract src/main.rs:42
~~~

You can even paste failing test output and it will extract needed files and AST out of it, like:

~~~bash
go test | probe extract
~~~

**Interactive AI Chat**
Use the AI assistant to ask questions about your codebase:

~~~bash
# Run directly with npx (no installation needed)
npx -y @buger/probe-chat

# Set your API key first
export ANTHROPIC_API_KEY=your_api_key
# Or for OpenAI
# export OPENAI_API_KEY=your_api_key

# Specify a directory to search (optional)
npx -y @buger/probe-chat /path/to/your/project
~~~

**Node.js SDK Usage**
Use Probe programmatically in your Node.js applications with the Vercel AI SDK:

~~~javascript
import { ProbeChat } from '@buger/probe-chat';
import { StreamingTextResponse } from 'ai';

// Create a chat instance
const chat = new ProbeChat({
  model: 'claude-3-sonnet-20240229',
  anthropicApiKey: process.env.ANTHROPIC_API_KEY,
  allowedFolders: ['/path/to/your/project']
});

// In an API route or Express handler
export async function POST(req) {
  const { messages } = await req.json();
  const userMessage = messages[messages.length - 1].content;
  
  // Get a streaming response from the AI
  const stream = await chat.chat(userMessage, { stream: true });
  
  // Return a streaming response
  return new StreamingTextResponse(stream);
}

// Or use it in a non-streaming way
const response = await chat.chat('How is authentication implemented?');
console.log(response);
~~~

**MCP server**

Integrate with any AI editor:

  ~~~json
  {
    "mcpServers": {
      "memory": {
        "command": "npx",
        "args": [
          "-y",
          "@buger/probe-mcp"
        ]
      }
    }
  }
  ~~~

Example queries:
> "Do the probe and search my codebase for implementations of the ranking algorithm"
>
> "Using probe find all functions related to error handling in the src directory"

---

## Features

- **AI-Friendly**: Extracts **entire functions, classes, or structs** so AI models get full context.
- **Fully Local**: Keeps your code on your machine—no external APIs.
- **Powered by ripgrep**: Extremely fast scanning of large codebases.
- **Tree-sitter Integration**: Parses and understands code structure accurately.
- **Re-Rankers & NLP**: Uses tokenization, stemming, BM25, TF-IDF, or hybrid ranking methods for better search results.
- **Code Extraction**: Extract specific code blocks or entire files with the `extract` command.
- **Multi-Language**: Works with popular languages like Rust, Python, JavaScript, TypeScript, Java, Go, C/C++, Swift, C#, and more.
- **Interactive AI Chat**: AI assistant example in the examples directory that can answer questions about your codebase using Claude or GPT models.
- **Flexible**: Run as a CLI tool, an MCP server, or an interactive AI chat.

---

## Installation

### Quick Installation

You can install Probe with a single command using npm, curl, or PowerShell:
You can install Probe with a single command using npm, curl, or PowerShell:

**Using npm (Recommended for Node.js users)**
~~~bash
npm install -g @buger/probe
~~~

**Using curl (For macOS and Linux)**
**Using curl (For macOS and Linux)**
~~~bash
curl -fsSL https://raw.githubusercontent.com/buger/probe/main/install.sh | bash
~~~

**What the curl script does**:

1. Detects your operating system and architecture
2. Fetches the latest release from GitHub
3. Downloads the appropriate binary for your system
4. Verifies the checksum for security
1. Detects your operating system and architecture
2. Fetches the latest release from GitHub
3. Downloads the appropriate binary for your system
4. Verifies the checksum for security
5. Installs the binary to `/usr/local/bin`

**Using PowerShell (For Windows)**
~~~powershell
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex
~~~

**What the PowerShell script does**:

1. Detects your system architecture (x86_64 or ARM64)
2. Fetches the latest release from GitHub
3. Downloads the appropriate Windows binary
4. Verifies the checksum for security
5. Installs the binary to your user directory (`%LOCALAPPDATA%\Probe`) by default
6. Provides instructions to add the binary to your PATH if needed

**Installation options**:

The PowerShell script supports several options:

~~~powershell
# Install for current user (default)
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex

# Install system-wide (requires admin privileges)
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--system"

# Install to a custom directory
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--dir", "C:\Tools\Probe"

# Show help
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--help"
~~~

**Using PowerShell (For Windows)**
~~~powershell
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex
~~~

**What the PowerShell script does**:

1. Detects your system architecture (x86_64 or ARM64)
2. Fetches the latest release from GitHub
3. Downloads the appropriate Windows binary
4. Verifies the checksum for security
5. Installs the binary to your user directory (`%LOCALAPPDATA%\Probe`) by default
6. Provides instructions to add the binary to your PATH if needed

**Installation options**:

The PowerShell script supports several options:

~~~powershell
# Install for current user (default)
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex

# Install system-wide (requires admin privileges)
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--system"

# Install to a custom directory
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--dir", "C:\Tools\Probe"

# Show help
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--help"
~~~

**Using PowerShell (For Windows)**
~~~powershell
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex
~~~

**What the PowerShell script does**:

1. Detects your system architecture (x86_64 or ARM64)
2. Fetches the latest release from GitHub
3. Downloads the appropriate Windows binary
4. Verifies the checksum for security
5. Installs the binary to your user directory (`%LOCALAPPDATA%\Probe`) by default
6. Provides instructions to add the binary to your PATH if needed

**Installation options**:

The PowerShell script supports several options:

~~~powershell
# Install for current user (default)
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex

# Install system-wide (requires admin privileges)
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--system"

# Install to a custom directory
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--dir", "C:\Tools\Probe"

# Show help
iwr -useb https://raw.githubusercontent.com/buger/probe/main/install.ps1 | iex -args "--help"
~~~

### Requirements

<<<<<<< HEAD
- **Operating Systems**: macOS, Linux, or Windows (with MSYS/Git Bash/WSL)  
- **Architectures**: x86_64 (all platforms) or ARM64 (macOS only)  
- **Tools**: `curl`, `bash`, and `sudo`/root privileges  
- **Windows-specific**: For building from source on Windows, you'll need:
  - MSYS2 with MinGW-w64 toolchain (provides GCC/G++)
  - Visual Studio 2022 Build Tools with C++ workload (optional, but recommended)
=======
- **Operating Systems**: macOS, Linux, or Windows
- **Architectures**: x86_64 (all platforms) or ARM64 (macOS and Windows)
- **Tools**:
  - For macOS/Linux: `curl`, `bash`, and `sudo`/root privileges
  - For Windows: PowerShell 5.1 or later
>>>>>>> dda6cb5a67b76f29cbd68ab3a16049b79ef96565

### Manual Installation

1. Download the appropriate binary for your platform from the [GitHub Releases](https://github.com/buger/probe/releases) page:
   - `probe-x86_64-linux.tar.gz` for Linux (x86_64)
   - `probe-x86_64-darwin.tar.gz` for macOS (Intel)
   - `probe-aarch64-darwin.tar.gz` for macOS (Apple Silicon)
   - `probe-x86_64-windows.zip` for Windows (x86_64)
   - `probe-aarch64-windows.zip` for Windows (ARM64)
   - `probe-x86_64-windows.zip` for Windows (x86_64)
   - `probe-aarch64-windows.zip` for Windows (ARM64)
   - `probe-x86_64-windows.zip` for Windows (x86_64)
   - `probe-aarch64-windows.zip` for Windows (ARM64)
2. Extract the archive:
   ~~~bash
   # For Linux/macOS
   tar -xzf probe-*-*.tar.gz
   
   # For Windows (using PowerShell)
   Expand-Archive -Path probe-*-windows.zip -DestinationPath .\probe
   ~~~
3. Move the binary to a location in your PATH:
   ~~~bash
   # For Linux/macOS
   sudo mv probe /usr/local/bin/
   
   # For Windows (using PowerShell)
   # Create a directory for the binary (if it doesn't exist)
   $installDir = "$env:LOCALAPPDATA\Probe"
   New-Item -ItemType Directory -Path $installDir -Force
   
   # Move the binary
   Move-Item -Path .\probe\probe.exe -Destination $installDir
   
   # Add to PATH (optional)
   [Environment]::SetEnvironmentVariable('PATH', [Environment]::GetEnvironmentVariable('PATH', 'User') + ";$installDir", 'User')
   # For Windows (using PowerShell)
   # Create a directory for the binary (if it doesn't exist)
   $installDir = "$env:LOCALAPPDATA\Probe"
   New-Item -ItemType Directory -Path $installDir -Force
   
   # Move the binary
   Move-Item -Path .\probe\probe.exe -Destination $installDir
   
   # Add to PATH (optional)
   [Environment]::SetEnvironmentVariable('PATH', [Environment]::GetEnvironmentVariable('PATH', 'User') + ";$installDir", 'User')
   ~~~

### Building from Source

1. Install Rust and Cargo (if not already installed):
   
   For macOS/Linux:
   
   For macOS/Linux:
   ~~~bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ~~~
   
<<<<<<< HEAD
   For Windows, visit [rustup.rs](https://rustup.rs/) and download the installer.
   
=======
   For Windows:
   ~~~powershell
   # Download and run the Rust installer
   Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile rustup-init.exe
   .\rustup-init.exe
   # Follow the on-screen instructions
   ~~~

>>>>>>> dda6cb5a67b76f29cbd68ab3a16049b79ef96565
2. Clone this repository:
   ~~~bash
   git clone https://github.com/buger/probe.git
   cd probe
   ~~~
   
3. Build the project:
   ~~~bash
   cargo build --release
   ~~~
   
   **Windows-specific build requirements:**
   
   If you encounter this error on Windows: 
   ```
   error occurred in cc-rs: failed to find tool "gcc.exe": program not found 
   (see https://docs.rs/cc/latest/cc/#compile-time-requirements for help)
   ```
   
   1. Install MSYS2 (provides GCC and other build tools):
      ~~~bash
      winget install MSYS2.MSYS2
      ~~~
   
   2. Open the "MSYS2 MSYS2" shortcut in Windows Start Menu and install the MinGW-w64 toolchain:
      ~~~bash
      pacman -S --needed base-devel mingw-w64-ucrt-x86_64-toolchain
      ~~~
   
   3. Add the MinGW tools to your PATH environment variable:
      - Open Windows Settings > System > About > Advanced system settings
      - Click "Environment Variables"
      - Edit the "Path" variable (either User or System variables)
      - Add `C:\msys64\ucrt64\bin` to the list
      - Click OK on all dialogs
   
   4. Restart your terminal/PowerShell and try building again
:
      ~~~bash
      cargo build --release
      ~~~

   **For more information:**
   - Official cc-rs documentation: https://docs.rs/cc/latest/cc/#compile-time-requirements
   - Microsoft C++ build tools: https://code.visualstudio.com/docs/cpp/config-mingw#_installing-the-mingww64-toolchain
   
4. (Optional) Install globally:
   ~~~bash
   cargo install --path .
   ~~~
   
   This will install the binary to your Cargo bin directory, which is typically:
   - `$HOME/.cargo/bin` on macOS/Linux
   - `%USERPROFILE%\.cargo\bin` on Windows
   
   This will install the binary to your Cargo bin directory, which is typically:
   - `$HOME/.cargo/bin` on macOS/Linux
   - `%USERPROFILE%\.cargo\bin` on Windows
   
   This will install the binary to your Cargo bin directory, which is typically:
   - `$HOME/.cargo/bin` on macOS/Linux
   - `%USERPROFILE%\.cargo\bin` on Windows

### Verifying the Installation

For macOS/Linux:
For macOS/Linux:
~~~bash
probe --version
~~~

For Windows:
~~~powershell
probe --version
~~~

If you get a "command not found" error on Windows, make sure the installation directory is in your PATH or use the full path to the executable:
~~~powershell
# If installed to the default user location
& "$env:LOCALAPPDATA\Probe\probe.exe" --version

# If installed to the default system location
& "$env:ProgramFiles\Probe\probe.exe" --version
~~~

For Windows:
~~~powershell
probe --version
~~~

If you get a "command not found" error on Windows, make sure the installation directory is in your PATH or use the full path to the executable:
~~~powershell
# If installed to the default user location
& "$env:LOCALAPPDATA\Probe\probe.exe" --version

# If installed to the default system location
& "$env:ProgramFiles\Probe\probe.exe" --version
~~~

### Troubleshooting

<<<<<<< HEAD
- **Permissions**: Ensure you can write to `/usr/local/bin`.  
- **Windows Build Errors**: If you see `gcc.exe: program not found` errors on Windows, follow the MSYS2 installation steps in the [Building from Source](#building-from-source) section.
- **System Requirements**: Double-check your OS/architecture.  
- **Manual Install**: If the quick install script fails, try [Manual Installation](#manual-installation).  
- **GitHub Issues**: Report issues on the [GitHub repository](https://github.com/buger/probe/issues).
=======
- **Permissions**:
  - For macOS/Linux: Ensure you can write to `/usr/local/bin`
  - For Windows: Ensure you have write permissions to the installation directory
- **System Requirements**: Double-check your OS/architecture compatibility
- **PATH Issues**:
  - For Windows: Restart your terminal after adding the installation directory to PATH
  - For macOS/Linux: Verify that `/usr/local/bin` is in your PATH
- **Manual Install**: If the quick install script fails, try [Manual Installation](#manual-installation)
- **GitHub Issues**: Report issues on the [GitHub repository](https://github.com/buger/probe/issues)
>>>>>>> dda6cb5a67b76f29cbd68ab3a16049b79ef96565
- **Permissions**:
  - For macOS/Linux: Ensure you can write to `/usr/local/bin`
  - For Windows: Ensure you have write permissions to the installation directory
- **System Requirements**: Double-check your OS/architecture compatibility
- **PATH Issues**:
  - For Windows: Restart your terminal after adding the installation directory to PATH
  - For macOS/Linux: Verify that `/usr/local/bin` is in your PATH
- **Manual Install**: If the quick install script fails, try [Manual Installation](#manual-installation)
- **GitHub Issues**: Report issues on the [GitHub repository](https://github.com/buger/probe/issues)

### Uninstalling

For macOS/Linux:
For macOS/Linux:
~~~bash
# If installed via npm
npm uninstall -g @buger/probe

# If installed via curl script or manually
sudo rm /usr/local/bin/probe
~~~

For Windows:
~~~powershell
# If installed via PowerShell script to default user location
Remove-Item -Path "$env:LOCALAPPDATA\Probe\probe.exe" -Force

# If installed via PowerShell script to system location
Remove-Item -Path "$env:ProgramFiles\Probe\probe.exe" -Force

# If you added the installation directory to PATH, you may want to remove it
# For user PATH:
$userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
$userPath = ($userPath -split ';' | Where-Object { $_ -ne "$env:LOCALAPPDATA\Probe" }) -join ';'
[Environment]::SetEnvironmentVariable('PATH', $userPath, 'User')
~~~

---
## Usage

Probe can be used in three main modes:

1. **CLI Mode**: Direct code search and extraction from the command line
2. **MCP Server Mode**: Run as a server exposing search functionality via MCP
3. **Web Interface**: Browser-based UI for code exploration

Additionally, there are example implementations in the examples directory:

- **AI Chat Example**: Interactive AI assistant for code exploration (in examples/chat)
- **Web Interface Example**: Browser-based UI for code exploration (in examples/web)

### CLI Mode

#### Search Command

~~~bash
probe search <SEARCH_PATTERN> [OPTIONS]
~~~

##### Key Options

- `<SEARCH_PATTERN>`: Pattern to search for (required)
- `--files-only`: Skip AST parsing; only list files with matches
- `--ignore`: Custom ignore patterns (in addition to `.gitignore`)
- `--exclude-filenames, -n`: Exclude files whose names match query words (filename matching is enabled by default)
- `--reranker, -r`: Choose a re-ranking algorithm (`hybrid`, `hybrid2`, `bm25`, `tfidf`)
- `--frequency, -s`: Frequency-based search (tokenization, stemming, stopword removal)
- `--exact`: Exact matching (overrides frequency search)
- `--max-results`: Maximum number of results to return
- `--max-bytes`: Maximum total bytes of code to return
- `--max-tokens`: Maximum total tokens of code to return (useful for AI)
- `--allow-tests`: Include test files and test code blocks
- `--any-term`: Match files containing **any** query terms (default behavior)
- `--no-merge`: Disable merging of adjacent code blocks after ranking (merging enabled by default)
- `--merge-threshold`: Max lines between code blocks to consider them adjacent for merging (default: 5)

##### Examples

~~~bash
# 1) Search for "setTools" in the current directory with frequency-based search
probe search "setTools"

# 2) Search for "impl" in ./src with exact matching
probe search "impl"  ./src --exact

# 3) Search for "keyword" returning only the top 5 results
probe search "keyword" --max-tokens 10000

# 4) Search for "function" and disable merging of adjacent code blocks
probe search "function" --no-merge
~~~

#### Extract Command

The extract command allows you to extract code blocks from files. When a line number is specified, it uses tree-sitter to find the closest suitable parent node (function, struct, class, etc.) for that line. You can also specify a symbol name to extract the code block for that specific symbol.

~~~bash
probe extract <FILES> [OPTIONS]
~~~

##### Key Options

- `<FILES>`: Files to extract from (can include line numbers with colon, e.g., `file.rs:10`, or symbol names with hash, e.g., `file.rs#function_name`)
- `--allow-tests`: Include test files and test code blocks in results
- `-c, --context <LINES>`: Number of context lines to include before and after the extracted block (default: 0)
- `-f, --format <FORMAT>`: Output format (`markdown`, `plain`, `json`) (default: `markdown`)

##### Examples

~~~bash
# 1) Extract a function containing line 42 from main.rs
probe extract src/main.rs:42

# 2) Extract multiple files or blocks
probe extract src/main.rs:42 src/lib.rs:15 src/cli.rs

# 3) Extract with JSON output format
probe extract src/main.rs:42 --format json

# 4) Extract with 5 lines of context around the specified line
probe extract src/main.rs:42 --context 5

# 5) Extract a specific function by name (using # symbol syntax)
probe extract src/main.rs#handle_extract

# 6) Extract a specific line range (using : syntax)
probe extract src/main.rs:10-20

# 7) Extract from stdin (useful with error messages or compiler output)
cat error_log.txt | probe extract
~~~

The extract command can also read file paths from stdin, making it useful for processing compiler errors or log files:

~~~bash
# Extract code blocks from files mentioned in error logs
grep -r "error" ./logs/ | probe extract
~~~

### MCP Server

Add the following to your AI editor's MCP configuration file:
  
  ~~~json
  {
    "mcpServers": {
      "memory": {
        "command": "npx",
        "args": [
          "-y",
          "@buger/probe-mcp"
        ]
      }
    }
  }
  ~~~
  
- **Example Usage in AI Editors**:
  
  Once configured, you can ask your AI assistant to search your codebase with natural language queries like:
  
  > "Do the probe and search my codebase for implementations of the ranking algorithm"
  >
  > "Using probe find all functions related to error handling in the src directory"

### AI Chat Mode

The AI chat functionality is available as a standalone npm package that can be run directly with npx.

#### Using npx (Recommended)

~~~bash
# Run directly with npx (no installation needed)
npx -y @buger/probe-chat

# Set your API key
export ANTHROPIC_API_KEY=your_api_key
# Or for OpenAI
# export OPENAI_API_KEY=your_api_key

# Or specify a directory to search
npx -y @buger/probe-chat /path/to/your/project
~~~

#### Using the npm package

~~~bash
# Install globally
npm install -g @buger/probe-chat

# Start the chat interface
probe-chat
~~~

#### Using the example code

~~~bash
# Navigate to the examples directory
cd examples/chat

# Install dependencies
npm install

# Set your API key
export ANTHROPIC_API_KEY=your_api_key
# Or for OpenAI
# export OPENAI_API_KEY=your_api_key

# Start the chat interface
node index.js
~~~

This starts an interactive CLI interface where you can ask questions about your codebase and get AI-powered responses.

#### Features

- **AI-Powered Search**: Uses LLMs to understand your questions and search the codebase intelligently
- **Multi-Model Support**: Works with both Anthropic's Claude and OpenAI's GPT models
- **Token Tracking**: Monitors token usage for both requests and responses
- **Conversation History**: Maintains context across multiple interactions
- **Colored Output**: Provides a user-friendly terminal interface with syntax highlighting

#### Configuration

Configure the chat using environment variables:

~~~bash
# Use Claude models (recommended)
export ANTHROPIC_API_KEY=your_api_key

# Or use OpenAI models
export OPENAI_API_KEY=your_api_key

# Override the default model
export MODEL_NAME=claude-3-opus-20240229

# Override API URLs (useful for proxies or enterprise deployments)
export ANTHROPIC_API_URL=https://your-anthropic-proxy.com
export OPENAI_API_URL=https://your-openai-proxy.com/v1

# Enable debug mode for detailed logging
export DEBUG=1
~~~

#### Example Usage

~~~
❯ How does the ranking algorithm work?

─ Response ─────────────────────────────────────────────────────────
I'll explain how the ranking algorithm works in the codebase.

The ranking system in Probe uses multiple algorithms to sort search results by relevance. The main ranking algorithms are:

1. TF-IDF (Term Frequency-Inverse Document Frequency)
2. BM25 (Best Matching 25)
3. Hybrid (a combination approach)

Let me break down each approach:

## TF-IDF Ranking

TF-IDF weighs terms based on how frequently they appear in a document versus how common they are across all documents.

Key implementation details:
- Term frequency (TF) measures how often a term appears in a document
- Inverse document frequency (IDF) measures how rare a term is across all documents
- Final score is calculated as TF × IDF

## BM25 Ranking

BM25 is an advanced ranking function that improves upon TF-IDF by adding document length normalization.

Key implementation details:
- Uses parameters k1 (term frequency saturation) and b (document length normalization)
- Handles edge cases like empty documents and rare terms
- Provides better results for longer documents

## Hybrid Ranking

The hybrid approach combines multiple ranking signals for better results:

1. Combines scores from both TF-IDF and BM25
2. Considers document length and term positions
3. Applies normalization to ensure fair comparison

The default reranker is "hybrid" which provides the best overall results for code search.

The ranking implementation can be found in `src/search/result_ranking.rs`.
─────────────────────────────────────────────────────────────────────
Token Usage: Request: 1245 Response: 1532 (Current message only: ~1532)
Total: 2777 tokens (Cumulative for entire session)
─────────────────────────────────────────────────────────────────────
~~~

### Web Interface

Probe includes a web-based chat interface that provides a user-friendly way to interact with your codebase using AI. You can run it directly with npx or set it up manually.

#### Quick Start with npx

~~~bash
# Run directly with npx (no installation needed)
npx -y @buger/probe-web

# Set your API key first
export ANTHROPIC_API_KEY=your_api_key

# Configure allowed folders (optional)
export ALLOWED_FOLDERS=/path/to/folder1,/path/to/folder2
~~~

#### Manual Setup and Configuration

1. **Navigate to the web directory**:
   ```bash
   cd web
   ```

2. **Install dependencies**:
   ```bash
   npm install
   ```

3. **Configure environment variables**:
   Create or edit the `.env` file in the web directory:
   ```
   ANTHROPIC_API_KEY=your_anthropic_api_key
   PORT=8080
   ALLOWED_FOLDERS=/path/to/folder1,/path/to/folder2
   ```

4. **Start the server**:
   ```bash
   npm start
   ```

5. **Access the web interface**:
   Open your browser and navigate to `http://localhost:8080`

#### Technical Details

- Built with vanilla JavaScript and Node.js
- Uses the Vercel AI SDK for Claude integration
- Executes Probe commands via the probeTool.js module
- Renders markdown with Marked.js and syntax highlighting with Highlight.js
- Supports Mermaid.js for diagram generation and visualization

---

## Supported Languages

Probe currently supports:

- **Rust** (`.rs`)
- **JavaScript / JSX** (`.js`, `.jsx`)
- **TypeScript / TSX** (`.ts`, `.tsx`)
- **Python** (`.py`)
- **Go** (`.go`)
- **C / C++** (`.c`, `.h`, `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`)
- **Java** (`.java`)
- **Ruby** (`.rb`)
- **PHP** (`.php`)
- **Swift** (`.swift`)
- **C#** (`.cs`)
- **Markdown** (`.md`, `.markdown`)

---

## How It Works

Probe combines **fast file scanning** with **deep code parsing** to provide highly relevant, context-aware results:

1. **Ripgrep Scanning**  
   Probe uses ripgrep to quickly search across your files, identifying lines that match your query. Ripgrep's efficiency allows it to handle massive codebases at lightning speed.

2. **AST Parsing with Tree-sitter**  
   For each file containing matches, Probe uses tree-sitter to parse the file into an Abstract Syntax Tree (AST). This process ensures that code blocks (functions, classes, structs) can be identified precisely.

3. **NLP & Re-Rankers**  
   Next, Probe applies classical NLP methods—tokenization, stemming, and stopword removal—alongside re-rankers such as **BM25**, **TF-IDF**, or the **hybrid** approach (combining multiple ranking signals). This step elevates the most relevant code blocks to the top, especially helpful for AI-driven searches.

4. **Block Extraction**  
   Probe identifies the smallest complete AST node containing each match (e.g., a full function or class). It extracts these code blocks and aggregates them into search results.

5. **Context for AI**  
   Finally, these structured blocks can be returned directly or fed into an AI system. By providing the full context of each code segment, Probe helps AI models navigate large codebases and produce more accurate insights.

---

## Adding Support for New Languages

1. **Tree-sitter Grammar**: In `Cargo.toml`, add the tree-sitter parser for the new language.  
2. **Language Module**: Create a new file in `src/language/` for parsing logic.  
3. **Implement Language Trait**: Adapt the parse method for the new language constructs.  
4. **Factory Update**: Register your new language in Probe's detection mechanism.

---

## Releasing New Versions

Probe uses GitHub Actions for multi-platform builds and releases.

1. **Update `Cargo.toml`** with the new version.  
2. **Create a new Git tag**:
   ~~~bash
   git tag -a vX.Y.Z -m "Release vX.Y.Z"
   git push origin vX.Y.Z
   ~~~
3. **GitHub Actions** will build, package, and draft a new release with checksums.

Each release includes:
- Linux binary (x86_64)
- macOS binaries (x86_64 and aarch64)
- Windows binaries (x86_64 and aarch64)
- SHA256 checksums

---

We believe that **local, privacy-focused, semantic code search** is essential for the future of AI-assisted development. Probe is built to empower developers and AI alike to navigate and comprehend large codebases more effectively.

For questions or contributions, please open an issue on [GitHub](https://github.com/buger/probe/issues). Happy coding—and searching!
