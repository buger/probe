#!/usr/bin/env node
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ErrorCode,
  ListToolsRequestSchema,
  McpError,
} from '@modelcontextprotocol/sdk/types.js';
import { exec } from 'child_process';
import { promisify } from 'util';
import path from 'path';
import fs from 'fs-extra';
import { fileURLToPath } from 'url';

// Import the probe package with type declarations
// @ts-ignore - Ignore missing type declarations for @buger/probe
import { search, query, extract, getBinaryPath, setBinaryPath } from '@buger/probe';

// Parse command-line arguments
function parseArgs(): { timeout?: number; lsp?: boolean } {
  const args = process.argv.slice(2);
  const config: { timeout?: number; lsp?: boolean } = {};
  
  for (let i = 0; i < args.length; i++) {
    if ((args[i] === '--timeout' || args[i] === '-t') && i + 1 < args.length) {
      const timeout = parseInt(args[i + 1], 10);
      if (!isNaN(timeout) && timeout > 0) {
        config.timeout = timeout;
        console.error(`Timeout set to ${timeout} seconds`);
      } else {
        console.error(`Invalid timeout value: ${args[i + 1]}. Using default.`);
      }
      i++; // Skip the next argument
    } else if (args[i] === '--lsp') {
      config.lsp = true;
      console.error('LSP mode enabled');
    } else if (args[i] === '--help' || args[i] === '-h') {
      console.log(`
Probe MCP Server

Usage:
  probe-mcp [options]

Options:
  --timeout, -t <seconds>  Set timeout for search operations (default: 30)
  --lsp                    Enable LSP (Language Server Protocol) for enhanced features
                           Automatically initializes language servers for the current workspace
  --help, -h              Show this help message
`);
      process.exit(0);
    }
  }
  
  return config;
}

const cliConfig = parseArgs();

const execAsync = promisify(exec);

// Get the package.json to determine the version
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Try multiple possible locations for package.json
let packageVersion = '0.0.0';
const possiblePaths = [
  path.resolve(__dirname, '..', 'package.json'),      // When installed from npm: build/../package.json
  path.resolve(__dirname, '..', '..', 'package.json') // In development: src/../package.json
];

for (const packageJsonPath of possiblePaths) {
  try {
    if (fs.existsSync(packageJsonPath)) {
      console.log(`Found package.json at: ${packageJsonPath}`);
      const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf-8'));
      if (packageJson.version) {
        packageVersion = packageJson.version;
        console.log(`Using version from package.json: ${packageVersion}`);
        break;
      }
    }
  } catch (error) {
    console.error(`Error reading package.json at ${packageJsonPath}:`, error);
  }
}

// If we still have 0.0.0, try to get version from npm package
if (packageVersion === '0.0.0') {
  try {
    // Try to get version from the package name itself
    const result = await execAsync('npm list -g @buger/probe-mcp --json');
    const npmList = JSON.parse(result.stdout);
    if (npmList.dependencies && npmList.dependencies['@buger/probe-mcp']) {
      packageVersion = npmList.dependencies['@buger/probe-mcp'].version;
      console.log(`Using version from npm list: ${packageVersion}`);
    }
  } catch (error) {
    console.error('Error getting version from npm:', error);
  }
}

import { existsSync } from 'fs';

// Get the path to the bin directory
const binDir = path.resolve(__dirname, '..', 'bin');
console.log(`Bin directory: ${binDir}`);

// The @buger/probe package now handles binary path management internally
// We don't need to manage the binary path in the MCP server anymore

interface SearchCodeArgs {
  path: string;
  query: string | string[];
  filesOnly?: boolean;
  ignore?: string[];
  excludeFilenames?: boolean;
  exact?: boolean;
  maxResults?: number;
  maxTokens?: number;
  allowTests?: boolean;
  session?: string;
  timeout?: number;
  noGitignore?: boolean;
  lsp?: boolean;
}

interface QueryCodeArgs {
  path: string;
  pattern: string;
  language?: string;
  ignore?: string[];
  allowTests?: boolean;
  maxResults?: number;
  format?: 'markdown' | 'plain' | 'json' | 'color';
  timeout?: number;
  noGitignore?: boolean;
}

interface ExtractCodeArgs {
  path: string;
  files: string[];
  allowTests?: boolean;
  contextLines?: number;
  format?: 'markdown' | 'plain' | 'json';
  timeout?: number;
  noGitignore?: boolean;
  lsp?: boolean;
}

class ProbeServer {
  private server: Server;
  private defaultTimeout: number;
  private lspEnabled: boolean;

  constructor(timeout: number = 30, lspEnabled: boolean = false) {
    this.defaultTimeout = timeout;
    this.lspEnabled = lspEnabled;
    this.server = new Server(
      {
        name: '@buger/probe-mcp',
        version: packageVersion,
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );

    this.setupToolHandlers();
    
    // Error handling
    this.server.onerror = (error) => console.error('[MCP Error]', error);
    process.on('SIGINT', async () => {
      await this.server.close();
      process.exit(0);
    });
  }

  private setupToolHandlers() {
    // Use the tool descriptions defined at the top of the file
    
    this.server.setRequestHandler(ListToolsRequestSchema, async () => ({
      tools: [
        {
          name: 'search_code',
          description: "Search code in the repository using ElasticSearch. Use this tool first for any code-related questions.",
          inputSchema: {
            type: 'object',
            properties: {
              path: {
                type: 'string',
                description: 'Absolute path to the directory to search in (e.g., "/Users/username/projects/myproject").',
              },
              query: {
                type: 'string',
                description: 'Elastic search query. Supports logical operators (AND, OR, NOT), and grouping with parentheses. Examples: "config", "(term1 OR term2) AND term3". Use quotes for exact matches, like function or type names.',
              },
              filesOnly: {
                type: 'boolean',
                description: 'Skip AST parsing and just output unique files',
              },
              ignore: {
                type: 'array',
                items: { type: 'string' },
                description: 'Custom patterns to ignore (in addition to .gitignore and common patterns)'
              },
              excludeFilenames: {
                type: 'boolean',
                description: 'Exclude filenames from being used for matching'
              },
              exact: {
                type: 'boolean',
                description: 'Perform exact search without tokenization (case-insensitive)'
              },
              allowTests: {
                type: 'boolean',
                description: 'Allow test files and test code blocks in results (disabled by default)'
              },
              session: {
                type: 'string',
                description: 'Session identifier for caching. Set to "new" if unknown, or want to reset cache. Re-use session ID returned from previous searches',
                default: "new",
              },
              timeout: {
                type: 'number',
                description: 'Timeout for the search operation in seconds (default: 30)',
              },
              noGitignore: {
                type: 'boolean',
                description: 'Skip .gitignore files (will use PROBE_NO_GITIGNORE environment variable if not set)',
              },
              lsp: {
                type: 'boolean',
                description: 'Use LSP (Language Server Protocol) for call hierarchy, reference counts, and enhanced symbol information',
              }
            },
            required: ['path', 'query']
          },
        },
        {
          name: 'query_code',
          description: "Search code using ast-grep structural pattern matching. Use this tool to find specific code structures like functions, classes, or methods.",
          inputSchema: {
            type: 'object',
            properties: {
              path: {
                type: 'string',
                description: 'Absolute path to the directory to search in (e.g., "/Users/username/projects/myproject").',
              },
              pattern: {
                type: 'string',
                description: 'The ast-grep pattern to search for. Examples: "fn $NAME($$$PARAMS) $$$BODY" for Rust functions, "def $NAME($$$PARAMS): $$$BODY" for Python functions.',
              },
              language: {
                type: 'string',
                description: 'The programming language to search in. If not specified, the tool will try to infer the language from file extensions. Supported languages: rust, javascript, typescript, python, go, c, cpp, java, ruby, php, swift, csharp.',
              },
              ignore: {
                type: 'array',
                items: { type: 'string' },
                description: 'Custom patterns to ignore (in addition to common patterns)',
              },
              maxResults: {
                type: 'number',
                description: 'Maximum number of results to return'
              },
              format: {
                type: 'string',
                enum: ['markdown', 'plain', 'json', 'color'],
                description: 'Output format for the query results'
              },
              timeout: {
                type: 'number',
                description: 'Timeout for the query operation in seconds (default: 30)',
              },
              noGitignore: {
                type: 'boolean',
                description: 'Skip .gitignore files (will use PROBE_NO_GITIGNORE environment variable if not set)',
              }
            },
            required: ['path', 'pattern']
          },
        },
        {
          name: 'extract_code',
          description: "Extract code blocks from files based on line number, or symbol name. Fetch full file when line number is not provided.",
          inputSchema: {
            type: 'object',
            properties: {
              path: {
                type: 'string',
                description: 'Absolute path to the directory to search in (e.g., "/Users/username/projects/myproject").',
              },
              files: {
                type: 'array',
                items: { type: 'string' },
                description: 'Files and lines or sybmbols to  extract from: /path/to/file.rs:10, /path/to/file.rs#func_name Path should be absolute.',
              },
              allowTests: {
                type: 'boolean',
                description: 'Allow test files and test code blocks in results (disabled by default)',
              },
              contextLines: {
                type: 'number',
                description: 'Number of context lines to include before and after the extracted block when AST parsing fails to find a suitable node',
                default: 0
              },
              format: {
                type: 'string',
                enum: ['markdown', 'plain', 'json'],
                description: 'Output format for the extracted code',
                default: 'markdown'
              },
              timeout: {
                type: 'number',
                description: 'Timeout for the extract operation in seconds (default: 30)',
              },
              noGitignore: {
                type: 'boolean',
                description: 'Skip .gitignore files (will use PROBE_NO_GITIGNORE environment variable if not set)',
              },
              lsp: {
                type: 'boolean',
                description: 'Use LSP (Language Server Protocol) for call hierarchy, reference counts, and enhanced symbol information',
              }
            },
            required: ['path', 'files'],
          },
        },
      ],
    }));

    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      if (request.params.name !== 'search_code' && request.params.name !== 'query_code' && request.params.name !== 'extract_code' &&
          request.params.name !== 'probe' && request.params.name !== 'query' && request.params.name !== 'extract') {
        throw new McpError(
          ErrorCode.MethodNotFound,
          `Unknown tool: ${request.params.name}`
        );
      }

      try {
        let result: string;
        
        // Log the incoming request for debugging
        console.error(`Received request for tool: ${request.params.name}`);
        console.error(`Request arguments: ${JSON.stringify(request.params.arguments)}`);
        
        // Handle both new tool names and legacy tool names
        if (request.params.name === 'search_code' || request.params.name === 'probe') {
          // Ensure arguments is an object
          if (!request.params.arguments || typeof request.params.arguments !== 'object') {
            throw new Error("Arguments must be an object");
          }
          
          const args = request.params.arguments as unknown as SearchCodeArgs;
          
          // Validate required fields
          if (!args.path) {
            throw new Error("Path is required in arguments");
          }
          if (!args.query) {
            throw new Error("Query is required in arguments");
          }
          
          result = await this.executeCodeSearch(args);
        } else if (request.params.name === 'query_code' || request.params.name === 'query') {
          const args = request.params.arguments as unknown as QueryCodeArgs;
          result = await this.executeCodeQuery(args);
        } else { // extract_code or extract
          const args = request.params.arguments as unknown as ExtractCodeArgs;
          result = await this.executeCodeExtract(args);
        }
        
        return {
          content: [
            {
              type: 'text',
              text: result,
            },
          ],
        };
      } catch (error) {
        console.error(`Error executing ${request.params.name}:`, error);
        return {
          content: [
            {
              type: 'text',
              text: `Error executing ${request.params.name}: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        };
      }
    });
  }

  private async executeCodeSearch(args: SearchCodeArgs): Promise<string> {
    try {
      // Ensure path is included in the options and is a non-empty string
      if (!args.path || typeof args.path !== 'string' || args.path.trim() === '') {
        throw new Error("Path is required and must be a non-empty string");
      }

      // Ensure query is included in the options
      if (!args.query) {
        throw new Error("Query is required");
      }

      // Log the arguments we received for debugging
      console.error(`Received search arguments: path=${args.path}, query=${JSON.stringify(args.query)}`);

      // Create a clean options object with only the essential properties first
      const options: any = {
        path: args.path.trim(),  // Ensure path is trimmed
        query: args.query
      };
      
      // Add optional parameters only if they exist
      if (args.filesOnly !== undefined) options.filesOnly = args.filesOnly;
      if (args.ignore !== undefined) options.ignore = args.ignore;
      if (args.excludeFilenames !== undefined) options.excludeFilenames = args.excludeFilenames;
      if (args.exact !== undefined) options.exact = args.exact;
      if (args.maxResults !== undefined) options.maxResults = args.maxResults;
      if (args.maxTokens !== undefined) options.maxTokens = args.maxTokens;
      if (args.allowTests !== undefined) options.allowTests = args.allowTests;
      // Use noGitignore from args, or fall back to PROBE_NO_GITIGNORE environment variable
      if (args.noGitignore !== undefined) {
        options.noGitignore = args.noGitignore;
      } else if (process.env.PROBE_NO_GITIGNORE) {
        options.noGitignore = process.env.PROBE_NO_GITIGNORE === 'true';
      }
      if (args.session !== undefined && args.session.trim() !== '') {
        options.session = args.session;
      } else {
        options.session = "new";
      }
      // Use timeout from args, or fall back to instance default
      if (args.timeout !== undefined) {
        options.timeout = args.timeout;
      } else if (this.defaultTimeout !== undefined) {
        options.timeout = this.defaultTimeout;
      }
      // Pass LSP flag if enabled globally or per-request
      if (args.lsp !== undefined) {
        options.lsp = args.lsp;
      } else if (this.lspEnabled) {
        options.lsp = true;
      }
      
      console.error("Executing search with options:", JSON.stringify(options, null, 2));
      
      // Double-check that path is still in the options object
      if (!options.path) {
        console.error("Path is missing from options object after construction");
        throw new Error("Path is missing from options object");
      }
      
      try {
        // Call search with the options object
        const result = await search(options);
        return result;
      } catch (searchError: any) {
        console.error("Search function error:", searchError);
        throw new Error(`Search function error: ${searchError.message || String(searchError)}`);
      }
    } catch (error: any) {
      console.error('Error executing code search:', error);
      throw new McpError(
        'MethodNotFound' as unknown as ErrorCode,
        `Error executing code search: ${error.message || String(error)}`
      );
    }
  }

  private async executeCodeQuery(args: QueryCodeArgs): Promise<string> {
    try {
      // Validate required parameters
      if (!args.path) {
        throw new Error("Path is required");
      }
      if (!args.pattern) {
        throw new Error("Pattern is required");
      }

      // Create a single options object with both pattern and path
      const options: any = {
        path: args.path,
        pattern: args.pattern,
        language: args.language,
        ignore: args.ignore,
        allowTests: args.allowTests,
        maxResults: args.maxResults,
        format: args.format,
        timeout: args.timeout || this.defaultTimeout
      };
      
      // Use noGitignore from args, or fall back to PROBE_NO_GITIGNORE environment variable
      if (args.noGitignore !== undefined) {
        options.noGitignore = args.noGitignore;
      } else if (process.env.PROBE_NO_GITIGNORE) {
        options.noGitignore = process.env.PROBE_NO_GITIGNORE === 'true';
      }
      
      console.log("Executing query with options:", JSON.stringify({
        path: options.path,
        pattern: options.pattern
      }));
      
      const result = await query(options);
      return result;
    } catch (error: any) {
      console.error('Error executing code query:', error);
      throw new McpError(
        'MethodNotFound' as unknown as ErrorCode,
        `Error executing code query: ${error.message || String(error)}`
      );
    }
  }

  private async executeCodeExtract(args: ExtractCodeArgs): Promise<string> {
    try {
      // Validate required parameters
      if (!args.path) {
        throw new Error("Path is required");
      }
      if (!args.files || !Array.isArray(args.files) || args.files.length === 0) {
        throw new Error("Files array is required and must not be empty");
      }

      // Create a single options object with files and other parameters
      const options: any = {
        files: args.files,
        path: args.path,
        allowTests: args.allowTests,
        contextLines: args.contextLines,
        format: args.format,
        timeout: args.timeout || this.defaultTimeout
      };
      
      // Use noGitignore from args, or fall back to PROBE_NO_GITIGNORE environment variable
      if (args.noGitignore !== undefined) {
        options.noGitignore = args.noGitignore;
      } else if (process.env.PROBE_NO_GITIGNORE) {
        options.noGitignore = process.env.PROBE_NO_GITIGNORE === 'true';
      }
      // Pass LSP flag if enabled globally or per-request
      if (args.lsp !== undefined) {
        options.lsp = args.lsp;
      } else if (this.lspEnabled) {
        options.lsp = true;
      }
      
      // Call extract with the complete options object
      try {
        // Track request size for token usage
        const requestSize = JSON.stringify(args).length;
        const requestTokens = Math.ceil(requestSize / 4); // Approximate token count
        
        // Execute the extract command
        const result = await extract(options);
        
        // Parse the result to extract token information if available
        let responseTokens = 0;
        let totalTokens = 0;
        
        // Try to extract token information from the result
        if (typeof result === 'string') {
          const tokenMatch = result.match(/Total tokens returned: (\d+)/);
          if (tokenMatch && tokenMatch[1]) {
            responseTokens = parseInt(tokenMatch[1], 10);
            totalTokens = requestTokens + responseTokens;
          }
          
          // Remove spinner debug output lines
          const cleanedLines = result.split('\n').filter(line =>
            !line.match(/^⠙|^⠹|^⠧|^⠇|^⠏/) &&
            !line.includes('Thinking...Extract:') &&
            !line.includes('Extract results:')
          );
          
          // Add token usage information if not already present
          if (!result.includes('Token Usage:')) {
            cleanedLines.push('');
            cleanedLines.push('Token Usage:');
            cleanedLines.push(`  Request tokens: ${requestTokens}`);
            cleanedLines.push(`  Response tokens: ${responseTokens}`);
            cleanedLines.push(`  Total tokens: ${totalTokens}`);
          }
          
          return cleanedLines.join('\n');
        }
        
        return result;
      } catch (error: any) {
        console.error(`Error extracting:`, error);
        return `Error extracting: ${error.message || String(error)}`;
      }
    } catch (error: any) {
      console.error('Error executing code extract:', error);
      throw new McpError(
        'MethodNotFound' as unknown as ErrorCode,
        `Error executing code extract: ${error.message || String(error)}`
      );
    }
  }

  async run() {
    // The @buger/probe package now handles binary path management internally
    // We don't need to verify or download the binary in the MCP server anymore
    
    // Initialize LSP servers for the current workspace if --lsp flag is enabled
    if (this.lspEnabled) {
      const workspaceRoot = process.cwd();
      console.error(`Initializing LSP servers for workspace: ${workspaceRoot}`);
      
      try {
        // Execute probe lsp init command to pre-warm language servers
        // Use recursive flag to discover nested projects in monorepos
        const initCmd = process.platform === 'win32' 
          ? `probe lsp init -w "${workspaceRoot}" --recursive`
          : `probe lsp init -w '${workspaceRoot}' --recursive`;
        
        const { stdout, stderr } = await execAsync(initCmd, {
          timeout: 10000, // 10 second timeout for initialization - don't wait too long
          env: { ...process.env }
        });
        
        if (stderr && !stderr.includes('Successfully initialized')) {
          console.error(`LSP initialization warnings: ${stderr}`);
        }
        
        console.error(`LSP servers initialized successfully for workspace: ${workspaceRoot}`);
        
        // Parse initialization output to show what was initialized
        if (stdout) {
          const lines = stdout.split('\n');
          const initializedServers = lines.filter(line => 
            line.includes('✓') || line.includes('language server')
          );
          if (initializedServers.length > 0) {
            console.error('Initialized language servers:');
            initializedServers.forEach(line => console.error(`  ${line.trim()}`));
          }
        }
      } catch (error: any) {
        // Don't fail MCP server startup if LSP initialization fails
        // LSP will still work with cold start on first use
        console.error(`Warning: Failed to initialize LSP servers: ${error.message || error}`);
        console.error('LSP features will still be available but may have slower first-use performance');
      }
    }
    
    // Just connect the server to the transport
    const transport = new StdioServerTransport();
    await this.server.connect(transport);
    console.error('Probe MCP server running on stdio');
  }
}

const server = new ProbeServer(cliConfig.timeout, cliConfig.lsp);
server.run().catch(console.error);
