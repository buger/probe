---
title: Language-Specific Indexing Guide
description: Detailed guide to how Probe indexes different programming languages
---

# Language-Specific Indexing Guide

This document provides detailed information about how Probe's LSP indexing system handles different programming languages, including language-specific configurations, features, and optimization strategies.

## Supported Languages Overview

| Language | Server | Status | Key Features | Initialization Time |
|----------|--------|---------|--------------|---------------------|
| **Rust** | rust-analyzer | Full | Macro expansion, trait resolution, cross-crate analysis | 10-30s |
| **TypeScript/JavaScript** | typescript-language-server | Full | Module resolution, type checking, JSX support | 5-15s |
| **Python** | pylsp | Full | Import analysis, type hints, virtual environments | 3-8s |
| **Go** | gopls | Full | Package awareness, interface satisfaction | 5-12s |
| **Java** | Eclipse JDT | Full | Classpath resolution, inheritance hierarchy | 15-45s |
| **C/C++** | clangd | Full | Header resolution, template instantiation | 8-20s |

## Language Details

### Rust

**Language Server**: rust-analyzer  
**Project Detection**: `Cargo.toml`  
**Workspace Scope**: Entire Cargo workspace including dependencies

#### Features

```rust
// rust-analyzer provides rich semantic understanding
pub fn calculate_fibonacci(n: usize) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => calculate_fibonacci(n - 1) + calculate_fibonacci(n - 2),
    }
}
```

**Supported Operations**:
- ✅ Call hierarchy (incoming/outgoing calls)
- ✅ Go to definition (cross-crate)
- ✅ Find all references (including macro expansions)
- ✅ Hover information (types, documentation)
- ✅ Workspace symbols
- ✅ Macro expansion analysis
- ✅ Trait resolution
- ✅ Generic type inference

#### Configuration

```json
// ./.probe/settings.json
{
  "indexing": {
    "language_configs": {
      "rust": {
        "enabled": true,
        "max_workers": 2,
        "memory_budget_mb": 512,
        "timeout_ms": 45000,
        "file_extensions": ["rs"],
        "exclude_patterns": [
          "**/target/**",
          "**/benches/**",
          "**/examples/**"
        ],
        "priority": 100,
        "features": {
          "extract_macros": true,
          "extract_traits": true,
          "extract_lifetimes": true,
          "extract_async": true
        }
      }
    }
  }
}
```

#### Environment Variables

```bash
# Rust-specific LSP settings
export PROBE_LSP_RUST_ANALYZER_PATH=/usr/local/bin/rust-analyzer
export PROBE_LSP_RUST_TIMEOUT=45000
export PROBE_LSP_RUST_MEMORY_MB=512

# Performance optimization
export RA_LOG=warn  # Reduce rust-analyzer logging
export RUST_BACKTRACE=0  # Disable backtraces for performance
```

#### Performance Characteristics

| Metric | Cold Start | Warm Cache | Notes |
|--------|------------|------------|-------|
| **Initialization** | 10-30s | N/A | Large projects take longer |
| **Call Hierarchy** | 200-2000ms | 2-5ms | Depends on project size |
| **Definition** | 50-500ms | 1-3ms | Cross-crate lookups slower |
| **References** | 100-1000ms | 3-8ms | Workspace-wide search |
| **Memory Usage** | 200-800MB | N/A | Large projects need more memory |

#### Optimization Tips

```bash
# Pre-build project for faster indexing
cargo check

# Use release mode for better performance
cargo build --release

# Limit target directories in workspace
probe lsp init-workspaces . --languages rust \
  --exclude-patterns "**/target/**,**/benches/**"

# Configure memory limits for large projects
export PROBE_LSP_MEMORY_LIMIT_MB=1024
```

#### Troubleshooting

**Slow initialization**:
```bash
# Check rust-analyzer logs
probe lsp logs --grep "rust-analyzer" -n 100

# Verify Cargo.toml is valid
cargo check

# Clear build cache if corrupt
rm -rf target/
```

**Missing dependencies**:
```bash
# Ensure dependencies are downloaded
cargo fetch

# Check for network issues
cargo update
```

### TypeScript/JavaScript

**Language Server**: typescript-language-server  
**Project Detection**: `package.json`, `tsconfig.json`  
**Workspace Scope**: npm/yarn workspace with node_modules

#### Features

```typescript
// Rich TypeScript analysis including JSX and modules
export interface ApiResponse<T> {
    data: T;
    status: number;
    message: string;
}

export async function fetchUserData(id: string): Promise<ApiResponse<User>> {
    const response = await fetch(`/api/users/${id}`);
    return handleApiResponse<User>(response);
}
```

**Supported Operations**:
- ✅ Call hierarchy (including imports/exports)
- ✅ Go to definition (across modules)
- ✅ Find all references
- ✅ Hover information (TypeScript types)
- ✅ Workspace symbols
- ✅ Module resolution
- ✅ JSX component analysis
- ✅ Type checking integration

#### Configuration

```json
// ./.probe/settings.json
{
  "indexing": {
    "language_configs": {
      "typescript": {
        "enabled": true,
        "max_workers": 2,
        "memory_budget_mb": 256,
        "timeout_ms": 25000,
        "file_extensions": ["ts", "tsx", "js", "jsx", "mjs"],
        "exclude_patterns": [
          "**/node_modules/**",
          "**/dist/**",
          "**/build/**",
          "**/.next/**",
          "**/coverage/**"
        ],
        "priority": 90,
        "features": {
          "extract_interfaces": true,
          "extract_decorators": true,
          "extract_jsx": true,
          "extract_types": true
        }
      }
    }
  }
}
```

#### Environment Variables

```bash
# TypeScript-specific settings
export PROBE_LSP_TYPESCRIPT_PATH=/usr/local/bin/typescript-language-server
export PROBE_LSP_TYPESCRIPT_TIMEOUT=25000
export PROBE_LSP_TYPESCRIPT_MEMORY_MB=256

# Node.js optimization
export NODE_OPTIONS="--max-old-space-size=2048"
```

#### Performance Characteristics

| Metric | Cold Start | Warm Cache | Notes |
|--------|------------|------------|-------|
| **Initialization** | 5-15s | N/A | Depends on node_modules size |
| **Call Hierarchy** | 100-800ms | 2-6ms | Module imports can be slow |
| **Definition** | 30-300ms | 1-4ms | Cross-module slower |
| **References** | 80-600ms | 3-10ms | Large projects slower |
| **Memory Usage** | 150-400MB | N/A | Scales with project size |

#### Optimization Tips

```bash
# Pre-install dependencies
npm install  # or yarn install

# Use TypeScript project references for large projects
# Configure tsconfig.json with "references"

# Exclude unnecessary directories
echo "node_modules/" >> .gitignore

# Configure memory for large projects
export NODE_OPTIONS="--max-old-space-size=4096"
```

### Python

**Language Server**: Python LSP Server (pylsp)  
**Project Detection**: `pyproject.toml`, `setup.py`, `requirements.txt`  
**Workspace Scope**: Python package with virtual environment support

#### Features

```python
# Python LSP provides rich analysis including type hints
from typing import List, Optional, Dict, Any
import asyncio

class UserRepository:
    def __init__(self, db_connection: str):
        self.db = db_connection
    
    async def find_user(self, user_id: int) -> Optional[Dict[str, Any]]:
        """Find user by ID with full type analysis."""
        return await self._query_database(user_id)
    
    async def _query_database(self, user_id: int) -> Optional[Dict[str, Any]]:
        # Implementation here
        pass
```

**Supported Operations**:
- ✅ Call hierarchy
- ✅ Go to definition (including imports)
- ✅ Find all references
- ✅ Hover information (docstrings, types)
- ✅ Workspace symbols
- ✅ Import analysis
- ✅ Type hint support
- ✅ Virtual environment detection

#### Configuration

```json
// ./.probe/settings.json
{
  "indexing": {
    "language_configs": {
      "python": {
        "enabled": true,
        "max_workers": 1,
        "memory_budget_mb": 128,
        "timeout_ms": 20000,
        "file_extensions": ["py", "pyi", "pyw"],
        "exclude_patterns": [
          "**/__pycache__/**",
          "**/venv/**",
          "**/.venv/**",
          "**/site-packages/**"
        ],
        "priority": 85,
        "features": {
          "extract_decorators": true,
          "extract_docstrings": true,
          "extract_async": true,
          "extract_type_hints": true
        }
      }
    }
  }
}
```

#### Environment Variables

```bash
# Python-specific settings
export PROBE_LSP_PYLSP_PATH=/usr/local/bin/pylsp
export PROBE_LSP_PYTHON_TIMEOUT=20000
export PROBE_LSP_PYTHON_MEMORY_MB=128

# Virtual environment
export VIRTUAL_ENV=/path/to/venv
export PYTHONPATH=/path/to/project
```

#### Virtual Environment Support

```bash
# Automatic virtual environment detection
# pylsp automatically detects and uses:
# - ./venv/
# - ./env/ 
# - $VIRTUAL_ENV
# - conda environments

# Manual virtual environment setup
source venv/bin/activate
probe lsp init-workspaces . --languages python

# Multiple Python versions
export PROBE_LSP_PYTHON_EXECUTABLE=/usr/bin/python3.11
```

#### Performance Characteristics

| Metric | Cold Start | Warm Cache | Notes |
|--------|------------|------------|-------|
| **Initialization** | 3-8s | N/A | Fast startup |
| **Call Hierarchy** | 80-400ms | 2-5ms | Simple hierarchy |
| **Definition** | 20-200ms | 1-3ms | Import resolution |
| **References** | 50-300ms | 2-6ms | Project-wide search |
| **Memory Usage** | 50-150MB | N/A | Lightweight |

### Go

**Language Server**: gopls  
**Project Detection**: `go.mod`  
**Workspace Scope**: Go module with dependency analysis

#### Features

```go
// gopls provides excellent Go analysis
package main

import (
    "context"
    "fmt"
    "net/http"
)

type UserService struct {
    client *http.Client
}

func (s *UserService) GetUser(ctx context.Context, id string) (*User, error) {
    return s.fetchFromAPI(ctx, "/users/" + id)
}

func (s *UserService) fetchFromAPI(ctx context.Context, path string) (*User, error) {
    // Implementation with full interface analysis
    return nil, nil
}
```

**Supported Operations**:
- ✅ Call hierarchy
- ✅ Go to definition (cross-package)
- ✅ Find all references
- ✅ Hover information (Go docs)
- ✅ Workspace symbols
- ✅ Package analysis
- ✅ Interface satisfaction
- ✅ Method set analysis

#### Configuration

```json
// ./.probe/settings.json
{
  "indexing": {
    "language_configs": {
      "go": {
        "enabled": true,
        "max_workers": 2,
        "memory_budget_mb": 256,
        "timeout_ms": 20000,
        "file_extensions": ["go"],
        "exclude_patterns": [
          "**/vendor/**",
          "**/*_test.go"
        ],
        "priority": 90,
        "features": {
          "extract_interfaces": true,
          "extract_channels": true,
          "extract_goroutines": true
        }
      }
    }
  }
}
```

#### Environment Variables

```bash
# Go-specific settings
export PROBE_LSP_GOPLS_PATH=/usr/local/bin/gopls
export PROBE_LSP_GO_TIMEOUT=20000
export PROBE_LSP_GO_MEMORY_MB=256

# Go environment
export GOPROXY=https://proxy.golang.org
export GOSUMDB=sum.golang.org
export GO111MODULE=on
```

### Java

**Language Server**: Eclipse JDT Language Server  
**Project Detection**: `pom.xml`, `build.gradle`, `.project`  
**Workspace Scope**: Maven/Gradle project with classpath resolution

#### Features

```java
// Java LSP provides comprehensive analysis
public class UserController {
    private final UserService userService;
    
    public UserController(UserService userService) {
        this.userService = userService;
    }
    
    @GetMapping("/users/{id}")
    public ResponseEntity<User> getUser(@PathVariable String id) {
        User user = userService.findById(id);
        return ResponseEntity.ok(user);
    }
}
```

**Supported Operations**:
- ✅ Call hierarchy (inheritance-aware)
- ✅ Go to definition (cross-jar)
- ✅ Find all references
- ✅ Hover information (Javadoc)
- ✅ Workspace symbols
- ✅ Classpath resolution
- ✅ Inheritance hierarchy
- ✅ Annotation processing

#### Configuration

```json
// ./.probe/settings.json
{
  "indexing": {
    "language_configs": {
      "java": {
        "enabled": true,
        "max_workers": 1,
        "memory_budget_mb": 512,
        "timeout_ms": 60000,
        "file_extensions": ["java"],
        "exclude_patterns": [
          "**/target/**",
          "**/build/**",
          "**/out/**"
        ],
        "priority": 85,
        "features": {
          "extract_annotations": true,
          "extract_generics": true,
          "extract_interfaces": true
        }
      }
    }
  }
}
```

### C/C++

**Language Server**: clangd  
**Project Detection**: `compile_commands.json`, `CMakeLists.txt`  
**Workspace Scope**: Compilation database scope

#### Features

```cpp
// clangd provides sophisticated C++ analysis
#include <memory>
#include <vector>

template<typename T>
class Repository {
private:
    std::vector<std::unique_ptr<T>> items;
    
public:
    void add(std::unique_ptr<T> item) {
        items.push_back(std::move(item));
    }
    
    T* find(const std::string& id) const {
        return findByPredicate([&id](const T& item) {
            return item.getId() == id;
        });
    }
};
```

**Supported Operations**:
- ✅ Call hierarchy
- ✅ Go to definition (cross-file)
- ✅ Find all references
- ✅ Hover information
- ✅ Workspace symbols
- ✅ Header resolution
- ✅ Template instantiation
- ✅ Macro expansion

#### Configuration

```json
// ./.probe/settings.json
{
  "indexing": {
    "language_configs": {
      "cpp": {
        "enabled": true,
        "max_workers": 2,
        "memory_budget_mb": 256,
        "timeout_ms": 25000,
        "file_extensions": ["c", "cpp", "cc", "cxx", "h", "hpp", "hxx"],
        "exclude_patterns": [
          "**/build/**",
          "**/cmake-build-*/**",
          "**/.cache/**"
        ],
        "priority": 80,
        "features": {
          "extract_templates": true,
          "extract_namespaces": true,
          "extract_classes": true
        }
      }
    }
  }
}
```

## Multi-Language Projects

### Workspace Detection

For projects with multiple languages:

```bash
# Initialize all detected languages
probe lsp init-workspaces . --recursive

# Example multi-language project structure:
my-project/
├── backend/          # Rust workspace
│   ├── Cargo.toml
│   └── src/
├── frontend/         # TypeScript workspace  
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
├── scripts/          # Python workspace
│   ├── pyproject.toml
│   └── *.py
└── mobile/           # Another TypeScript workspace
    ├── package.json
    └── src/
```

### Configuration Priorities

```json
// ./.probe/settings.json
{
  "indexing": {
    "priority_languages": ["rust", "typescript", "python"],
    "disabled_languages": ["java"],
    "language_configs": {
      "rust": {
        "memory_budget_mb": 512,
        "max_workers": 2
      },
      "typescript": {
        "memory_budget_mb": 256,
        "max_workers": 2
      }
    }
  }
}
```

## Language Server Troubleshooting

### Common Issues by Language

#### Rust Issues

```bash
# rust-analyzer not found
export PATH=$HOME/.cargo/bin:$PATH

# Slow indexing
cargo check  # Pre-build to speed up analysis

# Memory issues
export PROBE_LSP_MEMORY_LIMIT_MB=1024

# Build script issues
export PROBE_LSP_RUST_ANALYZER_PATH=/path/to/rust-analyzer
```

#### TypeScript Issues

```bash
# Missing dependencies
npm install

# TypeScript not found
npm install -g typescript

# Memory issues
export NODE_OPTIONS="--max-old-space-size=4096"

# Module resolution issues
# Check tsconfig.json paths configuration
```

#### Python Issues

```bash
# pylsp not found
pip install python-lsp-server

# Virtual environment not detected
source venv/bin/activate
export VIRTUAL_ENV=/path/to/venv

# Import resolution issues
export PYTHONPATH=/path/to/project
```

### Debug Commands

```bash
# Check language server status
probe lsp status --detailed

# View language-specific logs
probe lsp logs --grep "rust-analyzer"
probe lsp logs --grep "typescript"
probe lsp logs --grep "pylsp"

# Debug specific workspace
probe lsp debug workspace /path/to/project

# Test language server directly
probe lsp debug server rust
```

## Performance Optimization by Language

### Memory-Constrained Environments

```json
// Low-memory configuration
{
  "indexing": {
    "max_workers": 1,
    "memory_budget_mb": 128,
    "features": {
      "build_call_graph": false,
      "analyze_complexity": false,
      "extract_literals": false
    }
  }
}
```

### High-Performance Environments

```json
// High-performance configuration
{
  "indexing": {
    "max_workers": 8,
    "memory_budget_mb": 2048,
    "features": {
      "build_call_graph": true,
      "analyze_complexity": true,
      "extract_functions": true,
      "extract_types": true,
      "extract_variables": true,
      "extract_imports": true,
      "extract_docs": true
    },
    "lsp_caching": {
      "cache_during_indexing": true,
      "preload_common_symbols": true
    }
  }
}
```

## Next Steps

- **[Performance Guide](./indexing-performance.md)** - Detailed optimization strategies
- **[API Reference](./indexing-api-reference.md)** - Integration guide for developers
- **[Configuration Reference](./indexing-configuration.md)** - Complete configuration options