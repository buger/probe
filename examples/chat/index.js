#!/usr/bin/env node

// Check for non-interactive mode flag early, before any imports
// This ensures the environment variable is set before any module code runs
if (process.argv.includes('-m') || process.argv.includes('--message')) {
  process.env.PROBE_NON_INTERACTIVE = '1';
}

// Check if stdin is connected to a pipe (not a TTY)
// This allows for usage like: echo "query" | probe-chat
if (!process.stdin.isTTY) {
  process.env.PROBE_NON_INTERACTIVE = '1';
  process.env.PROBE_STDIN_PIPED = '1';
}

import 'dotenv/config';
import inquirer from 'inquirer';
import chalk from 'chalk';
import ora from 'ora';
import { Command } from 'commander';
import { existsSync, realpathSync, readFileSync } from 'fs';
import { resolve, dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { randomUUID } from 'crypto';
import { ProbeChat } from './probeChat.js';
import { TokenUsageDisplay } from './tokenUsageDisplay.js';
import { DEFAULT_SYSTEM_MESSAGE } from '@buger/probe';

/**
 * Main function that runs the Probe Chat CLI or web interface
 */
export function main() {
  // Get the directory name of the current module
  const __dirname = dirname(fileURLToPath(import.meta.url));
  const packageJsonPath = join(__dirname, 'package.json');

  // Read package.json to get the version
  let version = '1.0.0'; // Default fallback version
  try {
    const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
    version = packageJson.version || version;
  } catch (error) {
    // Non-critical, suppress in non-interactive unless debug
    // console.warn(`Warning: Could not read version from package.json: ${error.message}`);
  }

  // Create a new instance of the program
  const program = new Command();

  program
    .name('probe-chat')
    .description('CLI chat interface for Probe code search')
    .version(version)
    .option('-d, --debug', 'Enable debug mode')
    .option('--model-name <model>', 'Specify the model to use') // Renamed from --model
    .option('-f, --force-provider <provider>', 'Force a specific provider (options: anthropic, openai, google)')
    .option('-w, --web', 'Run in web interface mode')
    .option('-p, --port <port>', 'Port to run web server on (default: 8080)')
    .option('-m, --message <message>', 'Send a single message and exit (non-interactive mode)')
    .option('-s, --session-id <sessionId>', 'Specify a session ID for the chat (optional)')
    .option('--json', 'Output the response as JSON in non-interactive mode')
    .option('--max-iterations <number>', 'Maximum number of tool iterations allowed (default: 30)')
    .option('--prompt <value>', 'Use a custom prompt (values: architect, code-review, support, path to a file, or arbitrary string)')
    .option('--allow-edit', 'Enable the implement tool for editing files')
    .option('--allow-suggestions', 'Enable the suggest tool for suggesting code changes')
    .option('--trace-file [path]', 'Enable tracing to file (default: ./traces.jsonl)')
    .option('--trace-remote [endpoint]', 'Enable tracing to remote endpoint (default: http://localhost:4318/v1/traces)')
    .option('--trace-console', 'Enable tracing to console (for debugging)')
    .argument('[path]', 'Path to the codebase to search (overrides ALLOWED_FOLDERS)')
    .parse(process.argv);

  const options = program.opts();
  const pathArg = program.args[0];

  // --- Logging Configuration ---
  const isPipedInput = process.env.PROBE_STDIN_PIPED === '1';
  const isNonInteractive = !!options.message || isPipedInput;

  // Environment variable is already set at the top of the file
  // This is just for code clarity
  if (isNonInteractive && process.env.PROBE_NON_INTERACTIVE !== '1') {
    process.env.PROBE_NON_INTERACTIVE = '1';
  }

  // Raw logging for non-interactive output
  const rawLog = (...args) => console.log(...args);
  const rawError = (...args) => console.error(...args);

  // Disable color/formatting in raw non-interactive mode
  if (isNonInteractive && !options.json && !options.debug) {
    chalk.level = 0;
  }

  // Conditional logging helpers
  const logInfo = (...args) => {
    if (!isNonInteractive || options.debug) {
      console.log(...args);
    }
  };
  const logWarn = (...args) => {
    if (!isNonInteractive || options.debug) {
      console.warn(...args);
    } else if (isNonInteractive) {
      // Optionally log warnings to stderr in non-interactive mode even without debug
      // rawError('Warning:', ...args);
    }
  };
  const logError = (...args) => {
    // Always log errors, but use rawError in non-interactive mode
    if (isNonInteractive) {
      rawError('Error:', ...args); // Prefix with Error: for clarity on stderr
    } else {
      console.error(...args);
    }
  };
  // --- End Logging Configuration ---

  if (options.debug) {
    process.env.DEBUG_CHAT = '1';
    logInfo(chalk.yellow('Debug mode enabled'));
  }
  if (options.modelName) { // Use renamed option
    process.env.MODEL_NAME = options.modelName;
    logInfo(chalk.blue(`Using model: ${options.modelName}`));
  }
  if (options.forceProvider) {
    const provider = options.forceProvider.toLowerCase();
    if (!['anthropic', 'openai', 'google'].includes(provider)) {
      logError(chalk.red(`Invalid provider "${provider}". Must be one of: anthropic, openai, google`));
      process.exit(1);
    }
    process.env.FORCE_PROVIDER = provider;
    logInfo(chalk.blue(`Forcing provider: ${provider}`));
  }

  // Set MAX_TOOL_ITERATIONS from command line if provided
  if (options.maxIterations) {
    const maxIterations = parseInt(options.maxIterations, 10);
    if (isNaN(maxIterations) || maxIterations <= 0) {
      logError(chalk.red(`Invalid max iterations value: ${options.maxIterations}. Must be a positive number.`));
      process.exit(1);
    }
    process.env.MAX_TOOL_ITERATIONS = maxIterations.toString();
    logInfo(chalk.blue(`Setting maximum tool iterations to: ${maxIterations}`));
  }

  // Set ALLOW_EDIT from command line if provided
  if (options.allowEdit) {
    process.env.ALLOW_EDIT = '1';
    logInfo(chalk.blue(`Enabling implement tool with --allow-edit flag`));
  }

  // Set ALLOW_SUGGESTIONS from command line if provided
  if (options.allowSuggestions) {
    process.env.ALLOW_SUGGESTIONS = '1';
    logInfo(chalk.blue(`Enabling suggest tool with --allow-suggestions flag`));
  }

  // Set telemetry options from command line if provided
  if (options.traceFile !== undefined) {
    process.env.OTEL_ENABLE_FILE = 'true';
    process.env.OTEL_FILE_PATH = options.traceFile || './traces.jsonl';
    logInfo(chalk.blue(`Enabling file tracing to: ${process.env.OTEL_FILE_PATH}`));
  }
  
  if (options.traceRemote !== undefined) {
    process.env.OTEL_ENABLE_REMOTE = 'true';
    process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT = options.traceRemote || 'http://localhost:4318/v1/traces';
    logInfo(chalk.blue(`Enabling remote tracing to: ${process.env.OTEL_EXPORTER_OTLP_TRACES_ENDPOINT}`));
  }
  
  if (options.traceConsole) {
    process.env.OTEL_ENABLE_CONSOLE = 'true';
    logInfo(chalk.blue(`Enabling console tracing`));
  }


  // Handle custom prompt if provided
  let customPrompt = null;
  if (options.prompt) {
    // Check if it's one of the predefined prompts
    const predefinedPrompts = ['architect', 'code-review', 'support', 'engineer'];
    if (predefinedPrompts.includes(options.prompt)) {
      process.env.PROMPT_TYPE = options.prompt;
      logInfo(chalk.blue(`Using predefined prompt: ${options.prompt}`));
    } else {
      // Check if it's a file path
      try {
        const promptPath = resolve(options.prompt);
        if (existsSync(promptPath)) {
          customPrompt = readFileSync(promptPath, 'utf8');
          process.env.CUSTOM_PROMPT = customPrompt;
          logInfo(chalk.blue(`Loaded custom prompt from file: ${promptPath}`));
        } else {
          // Not a predefined prompt or existing file, treat as a direct string prompt
          customPrompt = options.prompt;
          process.env.CUSTOM_PROMPT = customPrompt;
          logInfo(chalk.blue(`Using custom prompt string`));
        }
      } catch (error) {
        // If there's an error resolving the path, treat as a direct string prompt
        customPrompt = options.prompt;
        process.env.CUSTOM_PROMPT = customPrompt;
        logInfo(chalk.blue(`Using custom prompt string`));
      }
    }
  }

  // Parse and validate allowed folders from environment variable
  const allowedFolders = process.env.ALLOWED_FOLDERS
    ? process.env.ALLOWED_FOLDERS.split(',').map(folder => folder.trim()).filter(Boolean)
    : [];

  // Resolve path argument to override ALLOWED_FOLDERS
  if (pathArg) {
    const resolvedPath = resolve(pathArg);
    if (existsSync(resolvedPath)) {
      const realPath = realpathSync(resolvedPath);
      process.env.ALLOWED_FOLDERS = realPath;
      logInfo(chalk.blue(`Using codebase path: ${realPath}`));
      // Clear allowedFolders if pathArg overrides it
      allowedFolders.length = 0;
      allowedFolders.push(realPath);
    } else {
      logError(chalk.red(`Path does not exist: ${resolvedPath}`));
      process.exit(1);
    }
  } else {
    // Log allowed folders only if interactive or debug
    logInfo('Configured search folders:');
    for (const folder of allowedFolders) {
      const exists = existsSync(folder);
      logInfo(`- ${folder} ${exists ? '✓' : '✗ (not found)'}`);
      if (!exists) {
        logWarn(chalk.yellow(`Warning: Folder "${folder}" does not exist or is not accessible`));
      }
    }
    if (allowedFolders.length === 0 && !isNonInteractive) { // Only warn if interactive
      logWarn(chalk.yellow('No folders configured. Set ALLOWED_FOLDERS in .env file or provide a path argument.'));
    }
  }


  // Set port for web server if specified
  if (options.port) {
    process.env.PORT = options.port;
  }

  // Check for API keys
  const anthropicApiKey = process.env.ANTHROPIC_API_KEY;
  const openaiApiKey = process.env.OPENAI_API_KEY;
  const googleApiKey = process.env.GOOGLE_API_KEY;
  const hasApiKeys = !!(anthropicApiKey || openaiApiKey || googleApiKey);

  // --- Non-Interactive Mode ---
  if (isNonInteractive) {
    if (!hasApiKeys) {
      logError(chalk.red('No API key provided. Please set ANTHROPIC_API_KEY, OPENAI_API_KEY, or GOOGLE_API_KEY environment variable.'));
      process.exit(1);
    }

    let chat;
    try {
      // Pass session ID if provided, ProbeChat generates one otherwise
      chat = new ProbeChat({
        sessionId: options.sessionId,
        isNonInteractive: true,
        customPrompt: customPrompt,
        promptType: options.prompt && ['architect', 'code-review', 'support', 'engineer'].includes(options.prompt) ? options.prompt : null,
        allowEdit: options.allowEdit,
        allowSuggestions: options.allowSuggestions
      });
      // Model/Provider info is logged via logInfo above if debug enabled
      logInfo(chalk.blue(`Using Session ID: ${chat.getSessionId()}`)); // Log the actual session ID being used
    } catch (error) {
      logError(chalk.red(`Initializing chat failed: ${error.message}`));
      process.exit(1);
    }

    // Function to read from stdin
    const readFromStdin = () => {
      return new Promise((resolve) => {
        let data = '';
        process.stdin.on('data', (chunk) => {
          data += chunk;
        });
        process.stdin.on('end', () => {
          resolve(data.trim());
        });
      });
    };

    // Async function to handle the single chat request
    const runNonInteractiveChat = async () => {
      try {
        // Get message from command line argument or stdin
        let message = options.message;

        // If no message argument but stdin is piped, read from stdin
        if (!message && isPipedInput) {
          logInfo('Reading message from stdin...'); // Log only if debug
          message = await readFromStdin();
        }

        if (!message) {
          logError('No message provided. Use --message option or pipe input to stdin.');
          process.exit(1);
        }

        logInfo('Sending message...'); // Log only if debug
        const result = await chat.chat(message, chat.getSessionId()); // Use the chat's current session ID

        if (result && typeof result === 'object' && result.response !== undefined) {
          if (options.json) {
            const outputData = {
              response: result.response,
              sessionId: chat.getSessionId(),
              tokenUsage: result.tokenUsage || null // Include usage if available
            };
            // Output JSON to stdout
            rawLog(JSON.stringify(outputData, null, 2));
          } else {
            // Output raw response text to stdout
            rawLog(result.response);
          }
          process.exit(0); // Success
        } else if (typeof result === 'string') { // Handle simple string responses (e.g., cancellation message)
          if (options.json) {
            rawLog(JSON.stringify({ response: result, sessionId: chat.getSessionId(), tokenUsage: null }, null, 2));
          } else {
            rawLog(result);
          }
          process.exit(0); // Exit cleanly
        }
        else {
          logError('Received an unexpected or empty response structure from chat.');
          if (options.json) {
            rawError(JSON.stringify({ error: 'Unexpected response structure', response: result, sessionId: chat.getSessionId() }, null, 2));
          }
          process.exit(1); // Error exit code
        }
      } catch (error) {
        logError(`Chat request failed: ${error.message}`);
        if (options.json) {
          // Output JSON error to stderr
          rawError(JSON.stringify({ error: error.message, sessionId: chat.getSessionId() }, null, 2));
        }
        process.exit(1); // Error exit code
      }
    };

    runNonInteractiveChat();
    return; // Exit main function, prevent interactive/web mode
  }
  // --- End Non-Interactive Mode ---


  // --- Web Mode ---
  if (options.web) {
    if (!hasApiKeys) {
      // Use logWarn for web mode warning
      logWarn(chalk.yellow('Warning: No API key provided. The web interface will show instructions on how to set up API keys.'));
    }
    // Import and start web server
    import('./webServer.js')
      .then(module => {
        const { startWebServer } = module;
        logInfo(`Starting web server on port ${process.env.PORT || 8080}...`);
        startWebServer(version, hasApiKeys, { allowEdit: options.allowEdit, allowSuggestions: options.allowSuggestions });
      })
      .catch(error => {
        logError(chalk.red(`Error starting web server: ${error.message}`));
        process.exit(1);
      });
    return; // Exit main function
  }
  // --- End Web Mode ---


  // --- Interactive CLI Mode ---
  // (This block only runs if not non-interactive and not web mode)

  if (!hasApiKeys) {
    // Use logError and standard console.log for setup instructions
    logError(chalk.red('No API key provided. Please set ANTHROPIC_API_KEY, OPENAI_API_KEY, or GOOGLE_API_KEY environment variable.'));
    console.log(chalk.cyan('You can find these instructions in the .env.example file:'));
    console.log(chalk.cyan('1. Create a .env file by copying .env.example'));
    console.log(chalk.cyan('2. Add your API key to the .env file'));
    console.log(chalk.cyan('3. Restart the application'));
    process.exit(1);
  }

  // Initialize ProbeChat for CLI mode
  let chat;
  try {
    // Pass session ID if provided (though less common for interactive start)
    chat = new ProbeChat({
      sessionId: options.sessionId,
      isNonInteractive: false,
      customPrompt: customPrompt,
      promptType: options.prompt && ['architect', 'code-review', 'support', 'engineer'].includes(options.prompt) ? options.prompt : null,
      allowEdit: options.allowEdit,
      allowSuggestions: options.allowSuggestions
    });

    // Log model/provider info using logInfo
    if (chat.apiType === 'anthropic') {
      logInfo(chalk.green(`Using Anthropic API with model: ${chat.model}`));
    } else if (chat.apiType === 'openai') {
      logInfo(chalk.green(`Using OpenAI API with model: ${chat.model}`));
    } else if (chat.apiType === 'google') {
      logInfo(chalk.green(`Using Google API with model: ${chat.model}`));
    }

    logInfo(chalk.blue(`Session ID: ${chat.getSessionId()}`));
    logInfo(chalk.cyan('Type "exit" or "quit" to end the chat'));
    logInfo(chalk.cyan('Type "usage" to see token usage statistics'));
    logInfo(chalk.cyan('Type "clear" to clear the chat history'));
    logInfo(chalk.cyan('-------------------------------------------'));
  } catch (error) {
    logError(chalk.red(`Error initializing chat: ${error.message}`));
    process.exit(1);
  }

  // Format AI response for interactive mode
  function formatResponseInteractive(response) {
    // Check if response is a structured object with response and tokenUsage properties
    let textResponse = '';
    if (response && typeof response === 'object' && 'response' in response) {
      textResponse = response.response;
    } else if (typeof response === 'string') {
      // Fallback for legacy format or simple string response
      textResponse = response;
    } else {
      return chalk.red('[Error: Invalid response format]');
    }

    // Apply formatting (e.g., highlighting tool calls)
    return textResponse.replace(
      /<tool_call>(.*?)<\/tool_call>/gs,
      (match, toolCall) => chalk.magenta(`[Tool Call] ${toolCall}`)
    );
  }

  // Main interactive chat loop
  async function startChat() {
    while (true) {
      const { message } = await inquirer.prompt([
        {
          type: 'input',
          name: 'message',
          message: chalk.blue('You:'),
          prefix: '',
        },
      ]);

      if (message.toLowerCase() === 'exit' || message.toLowerCase() === 'quit') {
        logInfo(chalk.yellow('Goodbye!'));
        break;
      } else if (message.toLowerCase() === 'usage') {
        const usage = chat.getTokenUsage();
        const display = new TokenUsageDisplay();
        const formatted = display.format(usage);

        // Use logInfo for usage details
        logInfo(chalk.blue('Current:', formatted.current.total));
        logInfo(chalk.blue('Context:', formatted.contextWindow));
        logInfo(chalk.blue('Cache:',
          `Read: ${formatted.current.cache.read},`,
          `Write: ${formatted.current.cache.write},`,
          `Total: ${formatted.current.cache.total}`));
        logInfo(chalk.blue('Total:', formatted.total.total));

        // Show context window in terminal title (only relevant for interactive)
        process.stdout.write('\x1B]0;Context: ' + formatted.contextWindow + '\x07');
        continue;
      } else if (message.toLowerCase() === 'clear') {
        const newSessionId = chat.clearHistory();
        logInfo(chalk.yellow('Chat history cleared'));
        logInfo(chalk.blue(`New session ID: ${newSessionId}`));
        continue;
      }

      const spinner = ora('Thinking...').start(); // Spinner is ok for interactive mode
      try {
        const result = await chat.chat(message); // Uses internal session ID
        spinner.stop();

        logInfo(chalk.green('Assistant:'));
        console.log(formatResponseInteractive(result)); // Use standard console.log for the actual response content
        console.log(); // Add a newline for readability

        // Update terminal title with context window size if available
        if (result && typeof result === 'object' && result.tokenUsage && result.tokenUsage.contextWindow) {
          process.stdout.write('\x1B]0;Context: ' + result.tokenUsage.contextWindow + '\x07');
        }
      } catch (error) {
        spinner.stop();
        logError(chalk.red(`Error: ${error.message}`)); // Use logError
      }
    }
  }

  startChat().catch((error) => {
    logError(chalk.red(`Fatal error in interactive chat: ${error.message}`));
    process.exit(1);
  });
  // --- End Interactive CLI Mode ---
}

// If this file is run directly, call main()
if (import.meta.url.startsWith('file:') && process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
