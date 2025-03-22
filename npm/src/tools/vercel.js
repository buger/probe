/**
 * Tools for Vercel AI SDK
 * @module tools/vercel
 */

import { tool } from 'ai';
import { search } from '../search.js';
import { query } from '../query.js';
import { extract } from '../extract.js';
import { searchSchema, querySchema, extractSchema, searchDescription, queryDescription, extractDescription } from './common.js';

/**
 * Search tool generator
 * 
 * @param {Object} [options] - Configuration options
 * @param {string} [options.sessionId] - Session ID for caching search results
 * @param {number} [options.maxTokens=10000] - Default max tokens
 * @param {boolean} [options.debug=false] - Enable debug logging
 * @returns {Object} Configured search tool
 */
export const searchTool = (options = {}) => {
	const { sessionId, maxTokens = 10000, debug = false } = options;

	return tool({
		name: 'search',
		description: searchDescription,
		parameters: searchSchema,
		execute: async ({ query: searchQuery, path, exact, allow_tests, maxTokens: paramMaxTokens }) => {
			try {
				// Use parameter maxTokens if provided, otherwise use the default
				const effectiveMaxTokens = paramMaxTokens || maxTokens;

				if (debug) {
					console.error(`Executing search with query: "${searchQuery}", path: "${path || '.'}", session: ${sessionId || 'none'}`);
				}

				const results = await search({
					query: searchQuery,
					path,
					exact,
					allow_tests,
					json: false,
					maxTokens: effectiveMaxTokens,
					session: sessionId // Pass session ID if provided
				});

				return results;
			} catch (error) {
				console.error('Error executing search command:', error);
				return `Error executing search command: ${error.message}`;
			}
		}
	});
};

/**
 * Query tool generator
 * 
 * @param {Object} [options] - Configuration options
 * @param {boolean} [options.debug=false] - Enable debug logging
 * @returns {Object} Configured query tool
 */
export const queryTool = (options = {}) => {
	const { debug = false } = options;

	return tool({
		name: 'query',
		description: queryDescription,
		parameters: querySchema,
		execute: async ({ pattern, path, language, allow_tests }) => {
			try {
				if (debug) {
					console.error(`Executing query with pattern: "${pattern}", path: "${path || '.'}", language: ${language || 'auto'}`);
				}

				const results = await query({
					pattern,
					path,
					language,
					allow_tests,
					json: false
				});

				return results;
			} catch (error) {
				console.error('Error executing query command:', error);
				return `Error executing query command: ${error.message}`;
			}
		}
	});
};

/**
 * Extract tool generator
 * 
 * @param {Object} [options] - Configuration options
 * @param {boolean} [options.debug=false] - Enable debug logging
 * @returns {Object} Configured extract tool
 */
export const extractTool = (options = {}) => {
	const { debug = false } = options;

	return tool({
		name: 'extract',
		description: extractDescription,
		parameters: extractSchema,
		execute: async ({ file_path, input_content, line, end_line, allow_tests, context_lines, format }) => {
			try {
				if (debug) {
					if (file_path) {
						console.error(`Executing extract with file: "${file_path}", context lines: ${context_lines || 10}`);
					} else if (input_content) {
						console.error(`Executing extract with input content, context lines: ${context_lines || 10}`);
					}
				}

				// Create a temporary file for input content if provided
				let tempFilePath = null;
				let extractOptions = {};

				if (input_content) {
					// Import required modules
					const { writeFileSync, unlinkSync } = await import('fs');
					const { join } = await import('path');
					const { tmpdir } = await import('os');
					const { randomUUID } = await import('crypto');

					// Create a temporary file with the input content
					tempFilePath = join(tmpdir(), `probe-extract-${randomUUID()}.txt`);
					writeFileSync(tempFilePath, input_content);

					if (debug) {
						console.error(`Created temporary file for input content: ${tempFilePath}`);
					}

					// Set up extract options with input file
					extractOptions = {
						inputFile: tempFilePath,
						allowTests: allow_tests,
						contextLines: context_lines,
						format
					};
				} else if (file_path) {
					// Parse file_path to handle line numbers and symbol names
					const files = [file_path];

					// Set up extract options with files
					extractOptions = {
						files,
						allowTests: allow_tests,
						contextLines: context_lines,
						format
					};
				} else {
					throw new Error('Either file_path or input_content must be provided');
				}

				// Execute the extract command
				const results = await extract(extractOptions);

				// Clean up temporary file if created
				if (tempFilePath) {
					const { unlinkSync } = await import('fs');
					try {
						unlinkSync(tempFilePath);
						if (debug) {
							console.error(`Removed temporary file: ${tempFilePath}`);
						}
					} catch (cleanupError) {
						console.error(`Warning: Failed to remove temporary file: ${cleanupError.message}`);
					}
				}

				return results;
			} catch (error) {
				console.error('Error executing extract command:', error);
				return `Error executing extract command: ${error.message}`;
			}
		}
	});
};