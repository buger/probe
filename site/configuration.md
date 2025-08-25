# Configuration

Probe supports a flexible multi-level configuration system that allows you to customize behavior globally, per-project, or locally.

## Configuration Files

Probe looks for configuration files in the following locations (in order of priority, highest to lowest):

1. **Environment Variables** - `PROBE_*` variables (highest priority)
2. **Local Settings** - `./.probe/settings.local.json` (project-specific, not committed to git)
3. **Project Settings** - `./.probe/settings.json` (project-wide, can be committed)
4. **Global Settings** - `~/.probe/settings.json` (user-wide settings)
5. **Built-in Defaults** (lowest priority)

Settings from higher priority sources override those from lower priority sources. Only explicitly set values are overridden - unset values inherit from lower priority levels.

## Configuration Merging

The configuration system uses deep merging:
- If global settings has `max_results: 50` and project settings has `max_tokens: 10000`, both values are kept
- If project settings has `max_results: 100`, it overrides the global value of `50`
- Local settings override both global and project settings for the same fields

## Example Configuration File

Copy `settings.example.json` from the repository root as a starting point:

```bash
# Global configuration (all projects)
cp settings.example.json ~/.probe/settings.json

# Project configuration (this project only)
cp settings.example.json ./.probe/settings.json

# Local configuration (not committed to git)
cp settings.example.json ./.probe/settings.local.json
```

## All Available Settings

### General Settings (`defaults`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `debug` | boolean | `false` | `PROBE_DEBUG` | Enable debug output |
| `log_level` | string | `"info"` | `PROBE_LOG_LEVEL` | Logging level: `error`, `warn`, `info`, `debug`, `trace` |
| `enable_lsp` | boolean | `false` | `PROBE_ENABLE_LSP` | Enable LSP features by default for all commands |
| `format` | string | `"color"` | `PROBE_FORMAT` | Default output format: `terminal`, `markdown`, `plain`, `json`, `xml`, `color` |
| `timeout` | number | `30` | `PROBE_TIMEOUT` | Default timeout in seconds for operations |

### Search Settings (`search`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `max_results` | number\|null | `null` | `PROBE_MAX_RESULTS` | Maximum number of search results to return (null = unlimited) |
| `max_tokens` | number\|null | `null` | `PROBE_MAX_TOKENS` | Maximum total tokens in search results for AI usage (null = unlimited) |
| `max_bytes` | number\|null | `null` | `PROBE_MAX_BYTES` | Maximum total bytes of code content to return (null = unlimited) |
| `frequency` | boolean | `true` | `PROBE_SEARCH_FREQUENCY` | Use frequency-based search with stemming and stopword removal |
| `reranker` | string | `"bm25"` | `PROBE_SEARCH_RERANKER` | Ranking algorithm: `bm25`, `hybrid`, `hybrid2`, `tfidf`, `ms-marco-tinybert`, `ms-marco-minilm-l6`, `ms-marco-minilm-l12` |
| `merge_threshold` | number | `5` | `PROBE_SEARCH_MERGE_THRESHOLD` | Maximum lines between code blocks to consider them adjacent for merging |
| `allow_tests` | boolean | `false` | `PROBE_ALLOW_TESTS` | Include test files and test code blocks in search results |
| `no_gitignore` | boolean | `false` | `PROBE_NO_GITIGNORE` | Ignore .gitignore files and patterns |

### Extract Settings (`extract`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `context_lines` | number | `0` | `PROBE_EXTRACT_CONTEXT_LINES` | Number of context lines to include before and after extracted blocks |
| `allow_tests` | boolean | `false` | `PROBE_ALLOW_TESTS` | Include test files and test code blocks in extraction results |

### Query Settings (`query`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `max_results` | number\|null | `null` | - | Maximum number of AST query results to return (null = unlimited) |
| `allow_tests` | boolean | `false` | `PROBE_ALLOW_TESTS` | Include test files in AST query results |

### LSP Settings (`lsp`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `include_stdlib` | boolean | `false` | `PROBE_LSP_INCLUDE_STDLIB` | Include standard library references in LSP results |
| `socket_path` | string\|null | `null` | `PROBE_LSP_SOCKET_PATH` | Custom path for LSP daemon socket (null = auto-detect) |
| `disable_autostart` | boolean | `false` | `PROBE_LSP_DISABLE_AUTOSTART` | Disable automatic LSP daemon startup |

### LSP Workspace Cache Settings (`lsp.workspace_cache`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `max_open_caches` | number | `8` | `PROBE_LSP_WORKSPACE_CACHE_MAX` | Maximum number of concurrent open workspace caches |
| `size_mb_per_workspace` | number | `100` | `PROBE_LSP_WORKSPACE_CACHE_SIZE_MB` | Size limit in MB per workspace cache |
| `lookup_depth` | number | `3` | `PROBE_LSP_WORKSPACE_LOOKUP_DEPTH` | Maximum parent directories to search for workspace markers |
| `base_dir` | string\|null | `null` | `PROBE_LSP_WORKSPACE_CACHE_DIR` | Custom base directory for workspace caches (null = auto-detect) |

### Performance Settings (`performance`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `tree_cache_size` | number | `2000` | `PROBE_TREE_CACHE_SIZE` | Maximum number of parsed syntax trees to cache in memory |
| `optimize_blocks` | boolean | `false` | `PROBE_OPTIMIZE_BLOCKS` | Enable experimental block extraction optimization |

### Indexing Settings (`indexing`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `enabled` | boolean | `true` | `PROBE_INDEXING_ENABLED` | Enable indexing subsystem (file-based indexing is default) |
| `auto_index` | boolean | `true` | `PROBE_INDEXING_AUTO_INDEX` | Auto-index workspaces when initialized |
| `watch_files` | boolean | `true` | `PROBE_INDEXING_WATCH_FILES` | Enable file watching for incremental indexing (auto-updates index on file changes) |
| `default_depth` | number | `3` | `PROBE_INDEXING_DEFAULT_DEPTH` | Default indexing depth for nested projects |
| `max_workers` | number | `8` | `PROBE_INDEXING_MAX_WORKERS` | Number of worker threads for indexing |
| `memory_budget_mb` | number | `512` | `PROBE_INDEXING_MEMORY_BUDGET_MB` | Memory budget in megabytes |
| `memory_pressure_threshold` | number | `0.8` | `PROBE_INDEXING_MEMORY_PRESSURE_THRESHOLD` | Memory pressure threshold (0.0-1.0) |
| `max_queue_size` | number | `10000` | `PROBE_INDEXING_MAX_QUEUE_SIZE` | Maximum queue size for pending files |
| `global_exclude_patterns` | array | See example | `PROBE_INDEXING_GLOBAL_EXCLUDE_PATTERNS` | File patterns to exclude (comma-separated in env) |
| `global_include_patterns` | array | `[]` | `PROBE_INDEXING_GLOBAL_INCLUDE_PATTERNS` | File patterns to include (comma-separated in env) |
| `max_file_size_mb` | number | `10` | `PROBE_INDEXING_MAX_FILE_SIZE_MB` | Maximum file size to index in MB |
| `incremental_mode` | boolean | `true` | `PROBE_INDEXING_INCREMENTAL_MODE` | Use incremental indexing based on file modification time |
| `discovery_batch_size` | number | `1000` | `PROBE_INDEXING_DISCOVERY_BATCH_SIZE` | Batch size for file discovery operations |
| `status_update_interval_secs` | number | `5` | `PROBE_INDEXING_STATUS_UPDATE_INTERVAL_SECS` | Interval between status updates |
| `file_processing_timeout_ms` | number | `30000` | `PROBE_INDEXING_FILE_PROCESSING_TIMEOUT_MS` | Timeout for processing a single file |
| `parallel_file_processing` | boolean | `true` | `PROBE_INDEXING_PARALLEL_FILE_PROCESSING` | Enable parallel processing within files |
| `persist_cache` | boolean | `false` | `PROBE_INDEXING_PERSIST_CACHE` | Cache parsed results to disk |
| `cache_directory` | string|null | `null` | `PROBE_INDEXING_CACHE_DIRECTORY` | Directory for persistent cache storage |
| `priority_languages` | array | `["rust", "typescript", "python"]` | `PROBE_INDEXING_PRIORITY_LANGUAGES` | Languages to index first (comma-separated in env) |
| `disabled_languages` | array | `[]` | `PROBE_INDEXING_DISABLED_LANGUAGES` | Languages to skip during indexing (comma-separated in env) |

### Indexing Features Settings (`indexing.features`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `extract_functions` | boolean | `true` | `PROBE_INDEXING_FEATURES_EXTRACT_FUNCTIONS` | Extract function and method signatures |
| `extract_types` | boolean | `true` | `PROBE_INDEXING_FEATURES_EXTRACT_TYPES` | Extract type definitions |
| `extract_variables` | boolean | `true` | `PROBE_INDEXING_FEATURES_EXTRACT_VARIABLES` | Extract variable declarations |
| `extract_imports` | boolean | `true` | `PROBE_INDEXING_FEATURES_EXTRACT_IMPORTS` | Extract import/export statements |
| `extract_docs` | boolean | `true` | `PROBE_INDEXING_FEATURES_EXTRACT_DOCS` | Extract documentation comments |
| `build_call_graph` | boolean | `false` | `PROBE_INDEXING_FEATURES_BUILD_CALL_GRAPH` | Build call graph relationships (expensive) |
| `extract_literals` | boolean | `false` | `PROBE_INDEXING_FEATURES_EXTRACT_LITERALS` | Extract string literals and constants |
| `analyze_complexity` | boolean | `false` | `PROBE_INDEXING_FEATURES_ANALYZE_COMPLEXITY` | Analyze complexity metrics |
| `extract_tests` | boolean | `true` | `PROBE_INDEXING_FEATURES_EXTRACT_TESTS` | Extract test-related symbols |
| `extract_error_handling` | boolean | `false` | `PROBE_INDEXING_FEATURES_EXTRACT_ERROR_HANDLING` | Extract error handling patterns |
| `extract_config` | boolean | `false` | `PROBE_INDEXING_FEATURES_EXTRACT_CONFIG` | Extract configuration code |
| `extract_database` | boolean | `false` | `PROBE_INDEXING_FEATURES_EXTRACT_DATABASE` | Extract database/ORM symbols |
| `extract_api_endpoints` | boolean | `false` | `PROBE_INDEXING_FEATURES_EXTRACT_API_ENDPOINTS` | Extract API endpoint definitions |
| `extract_security` | boolean | `false` | `PROBE_INDEXING_FEATURES_EXTRACT_SECURITY` | Extract security-related patterns |
| `extract_performance` | boolean | `false` | `PROBE_INDEXING_FEATURES_EXTRACT_PERFORMANCE` | Extract performance-critical sections |

### Indexing LSP Caching Settings (`indexing.lsp_caching`)

| Setting | Type | Default | Environment Variable | Description |
|---------|------|---------|---------------------|-------------|
| `cache_call_hierarchy` | boolean | `true` | `PROBE_INDEXING_LSP_CACHE_CALL_HIERARCHY` | Cache call hierarchy operations |
| `cache_definitions` | boolean | `false` | `PROBE_INDEXING_LSP_CACHE_DEFINITIONS` | Cache definition lookups |
| `cache_references` | boolean | `true` | `PROBE_INDEXING_LSP_CACHE_REFERENCES` | Cache reference lookups |
| `cache_hover` | boolean | `true` | `PROBE_INDEXING_LSP_CACHE_HOVER` | Cache hover information |
| `cache_document_symbols` | boolean | `false` | `PROBE_INDEXING_LSP_CACHE_DOCUMENT_SYMBOLS` | Cache document symbols |
| `cache_during_indexing` | boolean | `false` | `PROBE_INDEXING_LSP_CACHE_DURING_INDEXING` | Perform LSP operations during indexing |
| `preload_common_symbols` | boolean | `false` | `PROBE_INDEXING_LSP_PRELOAD_COMMON_SYMBOLS` | Preload cache with common operations |
| `max_cache_entries_per_operation` | number | `1000` | `PROBE_INDEXING_LSP_MAX_CACHE_ENTRIES_PER_OPERATION` | Max cache entries per operation type |
| `lsp_operation_timeout_ms` | number | `5000` | `PROBE_INDEXING_LSP_OPERATION_TIMEOUT_MS` | Timeout for LSP operations during indexing |
| `priority_operations` | array | `["call_hierarchy", "references", "hover"]` | `PROBE_INDEXING_LSP_PRIORITY_OPERATIONS` | Operations to prioritize (comma-separated in env) |
| `disabled_operations` | array | `[]` | `PROBE_INDEXING_LSP_DISABLED_OPERATIONS` | Operations to skip (comma-separated in env) |

## Example Configurations

### Enable LSP by Default

Create `~/.probe/settings.json`:
```json
{
  "defaults": {
    "enable_lsp": true
  }
}
```

Or set environment variable:
```bash
export PROBE_ENABLE_LSP=true
```

### Optimize for AI Usage

Create `./.probe/settings.json` in your project:
```json
{
  "search": {
    "max_tokens": 15000,
    "max_results": 50,
    "reranker": "hybrid"
  },
  "extract": {
    "context_lines": 3
  }
}
```

### Debug Configuration

Create `./.probe/settings.local.json` for local debugging:
```json
{
  "defaults": {
    "debug": true,
    "log_level": "debug"
  },
  "search": {
    "allow_tests": true
  }
}
```

### Monorepo Configuration

Global settings (`~/.probe/settings.json`):
```json
{
  "lsp": {
    "workspace_cache": {
      "max_open_caches": 16,
      "size_mb_per_workspace": 200
    }
  },
  "performance": {
    "tree_cache_size": 5000
  },
  "indexing": {
    "enabled": true,
    "max_workers": 16,
    "memory_budget_mb": 2048,
    "global_exclude_patterns": [
      "*.git/*",
      "*/node_modules/*",
      "*/target/*",
      "*/build/*",
      "*/dist/*"
    ]
  }
}
```

### Enable Indexing for Performance

Create `./.probe/settings.json` in your project:
```json
{
  "indexing": {
    "enabled": true,
    "auto_index": true,
    "features": {
      "build_call_graph": true,
      "analyze_complexity": true
    },
    "lsp_caching": {
      "cache_during_indexing": true,
      "preload_common_symbols": true
    }
  }
}
```

### Language-Specific Configuration

Configure specific languages in `settings.json`:
```json
{
  "indexing": {
    "language_configs": {
      "rust": {
        "enabled": true,
        "max_workers": 4,
        "memory_budget_mb": 1024,
        "timeout_ms": 45000,
        "file_extensions": ["rs"],
        "features": {
          "extract_macros": true,
          "extract_traits": true
        }
      },
      "typescript": {
        "enabled": true,
        "max_workers": 2,
        "memory_budget_mb": 512,
        "file_extensions": ["ts", "tsx"],
        "exclude_patterns": ["*.test.ts", "*.spec.ts"]
      }
    }
  }
}
```

## Configuration Commands

### View Current Configuration

```bash
# Show merged configuration as JSON
probe config show

# Show configuration as environment variables
probe config show --format env
```

### Validate Configuration

```bash
# Validate default configuration file
probe config validate

# Validate specific configuration file
probe config validate -f ./custom-settings.json
```

## Priority Example

Given these configuration files:

**Global** (`~/.probe/settings.json`):
```json
{
  "defaults": {
    "enable_lsp": true,
    "timeout": 60
  },
  "search": {
    "max_results": 50
  }
}
```

**Project** (`./.probe/settings.json`):
```json
{
  "defaults": {
    "format": "json"
  },
  "search": {
    "max_tokens": 15000,
    "reranker": "tfidf"
  }
}
```

**Local** (`./.probe/settings.local.json`):
```json
{
  "defaults": {
    "debug": true
  },
  "search": {
    "max_results": 100
  }
}
```

**Environment**:
```bash
export PROBE_TIMEOUT=120
```

**Result**:
- `debug`: `true` (from local)
- `log_level`: `"info"` (from defaults)
- `enable_lsp`: `true` (from global)
- `format`: `"json"` (from project)
- `timeout`: `120` (from environment, overrides global's 60)
- `max_results`: `100` (from local, overrides global's 50)
- `max_tokens`: `15000` (from project)
- `reranker`: `"tfidf"` (from project)

## Tips

1. **Use global settings** for personal preferences that apply to all projects
2. **Use project settings** for team-wide configuration that should be committed to version control
3. **Use local settings** for temporary overrides or personal preferences specific to one project
4. **Use environment variables** for CI/CD pipelines or temporary overrides

## Migration from Previous Versions

If you were using the old configuration system:
1. Move `~/.config/probe/config.json` to `~/.probe/settings.json`
2. Convert any `.probe-lsp.toml` or `indexing.toml` files to the new `settings.json` format
3. Update any references from `config.json` to `settings.json`
4. Environment variables remain the same (all `PROBE_*` variables still work)

### Converting from TOML to JSON

Old TOML format (`.probe-lsp.toml` or `indexing.toml`):
```toml
[indexing]
enabled = true
auto_index = true
max_workers = 8

[indexing.features]
extract_functions = true
build_call_graph = false

[indexing.language_configs.rust]
enabled = true
max_workers = 4
memory_budget_mb = 1024
```

New JSON format (`settings.json`):
```json
{
  "indexing": {
    "enabled": true,
    "auto_index": true,
    "max_workers": 8,
    "features": {
      "extract_functions": true,
      "build_call_graph": false
    },
    "language_configs": {
      "rust": {
        "enabled": true,
        "max_workers": 4,
        "memory_budget_mb": 1024
      }
    }
  }
}
```