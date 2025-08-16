/**
 * Extract functionality for the probe package
 * @module extract
 */

import { exec } from 'child_process';
import { promisify } from 'util';
import { getBinaryPath, buildCliArgs, escapeString } from './utils.js';

const execAsync = promisify(exec);

/**
 * Flag mapping for extract options
 * Maps option keys to command-line flags
 */
const EXTRACT_FLAG_MAP = {
	allowTests: '--allow-tests',
	contextLines: '--context',
	format: '--format',
	inputFile: '--input-file',
	lsp: '--lsp'
};

/**
 * Extract code blocks from files
 * 
 * @param {Object} options - Extract options
 * @param {string[]} [options.files] - Files to extract from (can include line numbers with colon, e.g., "/path/to/file.rs:10")
 * @param {string} [options.inputFile] - Path to a file containing unstructured text to extract file paths from
 * @param {boolean} [options.allowTests] - Include test files
 * @param {number} [options.contextLines] - Number of context lines to include
 * @param {string} [options.format] - Output format ('markdown', 'plain', 'json')
 * @param {boolean} [options.lsp] - Use LSP (Language Server Protocol) for call hierarchy and reference graphs
 * @param {Object} [options.binaryOptions] - Options for getting the binary
 * @param {boolean} [options.binaryOptions.forceDownload] - Force download even if binary exists
 * @param {string} [options.binaryOptions.version] - Specific version to download
 * @param {boolean} [options.json] - Return results as parsed JSON instead of string
 * @returns {Promise<string|Object>} - Extracted code as string or parsed JSON
 * @throws {Error} If the extraction fails
 */
export async function extract(options) {
	if (!options) {
		throw new Error('Options object is required');
	}

	// Either files or inputFile must be provided
	if ((!options.files || !Array.isArray(options.files) || options.files.length === 0) && !options.inputFile) {
		throw new Error('Either files array or inputFile must be provided');
	}

	// Get the binary path
	const binaryPath = await getBinaryPath(options.binaryOptions || {});

	// Build CLI arguments from options
	const cliArgs = buildCliArgs(options, EXTRACT_FLAG_MAP);

	// If json option is true, override format to json
	if (options.json && !options.format) {
		cliArgs.push('--format', 'json');
	}

	// Add files as positional arguments if provided
	if (options.files && Array.isArray(options.files) && options.files.length > 0) {
		for (const file of options.files) {
			cliArgs.push(escapeString(file));
		}
	}

	// Create a single log record with all extract parameters
	let logMessage = `\nExtract:`;
	if (options.files && options.files.length > 0) {
		logMessage += ` files="${options.files.join(', ')}"`;
	}
	if (options.inputFile) logMessage += ` inputFile="${options.inputFile}"`;
	if (options.allowTests) logMessage += " allowTests=true";
	if (options.contextLines) logMessage += ` contextLines=${options.contextLines}`;
	if (options.format) logMessage += ` format=${options.format}`;
	if (options.json) logMessage += " json=true";
	console.error(logMessage);

	// Execute command
	const command = `${binaryPath} extract ${cliArgs.join(' ')}`;

	try {
		const { stdout, stderr } = await execAsync(command);

		if (stderr) {
			console.error(`stderr: ${stderr}`);
		}

		// Parse the output to extract token usage information
		let tokenUsage = {
			requestTokens: 0,
			responseTokens: 0,
			totalTokens: 0
		};

		// Calculate approximate request tokens
		if (options.files && Array.isArray(options.files)) {
			tokenUsage.requestTokens = options.files.join(' ').length / 4;
		} else if (options.inputFile) {
			tokenUsage.requestTokens = options.inputFile.length / 4;
		}

		// Try to extract token information from the output
		if (stdout.includes('Total tokens returned:')) {
			const tokenMatch = stdout.match(/Total tokens returned: (\d+)/);
			if (tokenMatch && tokenMatch[1]) {
				tokenUsage.responseTokens = parseInt(tokenMatch[1], 10);
				tokenUsage.totalTokens = tokenUsage.requestTokens + tokenUsage.responseTokens;
			}
		}

		// Add token usage information to the output
		let output = stdout;

		// Add token usage information at the end if not already present
		if (!output.includes('Token Usage:')) {
			output += `\nToken Usage:\n  Request tokens: ${tokenUsage.requestTokens}\n  Response tokens: ${tokenUsage.responseTokens}\n  Total tokens: ${tokenUsage.totalTokens}\n`;
		}

		// Parse JSON if requested or if format is json
		if (options.json || options.format === 'json') {
			try {
				const jsonOutput = JSON.parse(stdout);

				// Add token usage to JSON output
				if (!jsonOutput.token_usage) {
					jsonOutput.token_usage = {
						request_tokens: tokenUsage.requestTokens,
						response_tokens: tokenUsage.responseTokens,
						total_tokens: tokenUsage.totalTokens
					};
				}

				return jsonOutput;
			} catch (error) {
				console.error('Error parsing JSON output:', error);
				return output; // Fall back to string output with token usage
			}
		}

		return output;
	} catch (error) {
		// Enhance error message with command details
		const errorMessage = `Error executing extract command: ${error.message}\nCommand: ${command}`;
		throw new Error(errorMessage);
	}
}