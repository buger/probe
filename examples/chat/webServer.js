import 'dotenv/config';
import { createServer } from 'http';
import { streamText } from 'ai';
import { readFileSync, existsSync } from 'fs';
import { resolve, dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { randomUUID } from 'crypto';
import { ProbeChat } from './probeChat.js';
import { authMiddleware, withAuth } from './auth.js';
import {
	probeTool,
	searchToolInstance,
	queryToolInstance,
	extractToolInstance,
	toolCallEmitter,
	cancelToolExecutions,
	clearToolExecutionData,
	isSessionCancelled
} from './probeTool.js';
import { registerRequest, cancelRequest, clearRequest, isRequestActive } from './cancelRequest.js';

// Get the directory name of the current module
const __dirname = dirname(fileURLToPath(import.meta.url));

/**
 * Start the web server
 * @param {string} version - The version of the application
 * @param {boolean} hasApiKeys - Whether any API keys are configured
 */
export function startWebServer(version, hasApiKeys = true) {
	// Authentication configuration
	const AUTH_ENABLED = process.env.AUTH_ENABLED === '1';
	const AUTH_USERNAME = process.env.AUTH_USERNAME || 'admin';
	const AUTH_PASSWORD = process.env.AUTH_PASSWORD || 'password';

	if (AUTH_ENABLED) {
		console.log(`Authentication enabled (username: ${AUTH_USERNAME})`);
	} else {
		console.log('Authentication disabled');
	}

	// Map to store SSE clients by session ID
	const sseClients = new Map();

	// Initialize the ProbeChat instance if API keys are available
	let probeChat;
	let noApiKeysMode = false;

	if (hasApiKeys) {
		try {
			// Generate a default session ID for the server
			const defaultSessionId = randomUUID();
			console.log(`Generated default session ID: ${defaultSessionId}`);

			probeChat = new ProbeChat({
				sessionId: defaultSessionId // Use the default session ID
			});
			console.log(`Server initialized with session ID: ${probeChat.getSessionId()}`);
		} catch (error) {
			console.error('Error initializing ProbeChat:', error.message);
			noApiKeysMode = true;
			console.log('Running in No API Keys mode - will show setup instructions to users');
		}
	} else {
		noApiKeysMode = true;
		console.log('Running in No API Keys mode - will show setup instructions to users');
	}

	// Define the tools available to the AI (only if we have API keys)
	const tools = noApiKeysMode ? [] : [probeTool, searchToolInstance, queryToolInstance, extractToolInstance];

	// Track token usage for monitoring
	let totalRequestTokens = 0;
	let totalResponseTokens = 0;

	/**
	 * Handle non-streaming chat request (returns complete response as JSON)
	 */
	async function handleNonStreamingChatRequest(req, res, message, sessionId) {
		try {
			const DEBUG = process.env.DEBUG === '1';
			if (DEBUG) {
				console.log(`\n[DEBUG] ===== API Chat Request (non-streaming) =====`);
				console.log(`[DEBUG] User message: "${message}"`);
			}

			// Use the ProbeChat instance to get a response
			// If a session ID was provided, use it to track the conversation
			const responseText = await probeChat.chat(message, sessionId);

			// Get token usage
			const tokenUsage = probeChat.getTokenUsage();
			totalRequestTokens = tokenUsage.request;
			totalResponseTokens = tokenUsage.response;

			// Return response as JSON
			res.writeHead(200, { 'Content-Type': 'application/json' });
			res.end(JSON.stringify({
				response: responseText,
				tokenUsage: tokenUsage,
				timestamp: new Date().toISOString()
			}));

			console.log('Finished generating non-streaming response');
		} catch (error) {
			console.error('Error generating response:', error);

			// Determine the appropriate status code and error message
			let statusCode = 500;
			let errorMessage = 'Internal server error';

			if (error.status) {
				// Handle API-specific error codes
				statusCode = error.status;

				// Provide more specific error messages based on status code
				if (statusCode === 401) {
					errorMessage = 'Authentication failed: Invalid API key';
				} else if (statusCode === 403) {
					errorMessage = 'Authorization failed: Insufficient permissions';
				} else if (statusCode === 404) {
					errorMessage = 'Resource not found: Check API endpoint URL';
				} else if (statusCode === 429) {
					errorMessage = 'Rate limit exceeded: Too many requests';
				} else if (statusCode >= 500) {
					errorMessage = 'API server error: Service may be unavailable';
				}
			} else if (error.code === 'ENOTFOUND' || error.code === 'ECONNREFUSED') {
				// Handle connection errors
				statusCode = 503;
				errorMessage = 'Connection failed: Unable to reach API server';
			} else if (error.message && error.message.includes('timeout')) {
				statusCode = 504;
				errorMessage = 'Request timeout: API server took too long to respond';
			}

			res.writeHead(statusCode, { 'Content-Type': 'application/json' });
			res.end(JSON.stringify({
				error: errorMessage,
				message: error.message,
				status: statusCode
			}));
		}
	}

	/**
	 * Handle streaming chat request (returns chunks of text)
	 */
	async function handleStreamingChatRequest(req, res, message, sessionId) {
		try {
			const DEBUG = process.env.DEBUG === '1';
			if (DEBUG) {
				console.log(`\n[DEBUG] ===== API Chat Request (streaming) =====`);
				console.log(`[DEBUG] User message: "${message}"`);
			}

			res.writeHead(200, {
				'Content-Type': 'text/plain',
				'Transfer-Encoding': 'chunked',
				'Cache-Control': 'no-cache',
				'Connection': 'keep-alive'
			});

			// Use the ProbeChat instance to get a response
			// If a session ID was provided, use it to track the conversation
			const responseText = await probeChat.chat(message, sessionId);

			// Get token usage
			const tokenUsage = probeChat.getTokenUsage();
			totalRequestTokens = tokenUsage.request;
			totalResponseTokens = tokenUsage.response;

			// Write the response as a single chunk
			res.write(responseText);
			res.end();

			console.log('Finished streaming response');
		} catch (error) {
			console.error('Error streaming response:', error);

			// Determine the appropriate status code and error message
			let statusCode = 500;
			let errorMessage = 'Internal server error';

			if (error.status) {
				// Handle API-specific error codes
				statusCode = error.status;

				// Provide more specific error messages based on status code
				if (statusCode === 401) {
					errorMessage = 'Authentication failed: Invalid API key';
				} else if (statusCode === 403) {
					errorMessage = 'Authorization failed: Insufficient permissions';
				} else if (statusCode === 404) {
					errorMessage = 'Resource not found: Check API endpoint URL';
				} else if (statusCode === 429) {
					errorMessage = 'Rate limit exceeded: Too many requests';
				} else if (statusCode >= 500) {
					errorMessage = 'API server error: Service may be unavailable';
				}
			} else if (error.code === 'ENOTFOUND' || error.code === 'ECONNREFUSED') {
				// Handle connection errors
				statusCode = 503;
				errorMessage = 'Connection failed: Unable to reach API server';
			} else if (error.message && error.message.includes('timeout')) {
				statusCode = 504;
				errorMessage = 'Request timeout: API server took too long to respond';
			}

			// For streaming responses, we need to send a plain text error
			res.writeHead(statusCode, { 'Content-Type': 'text/plain' });
			res.end(`Error: ${errorMessage} - ${error.message}`);
		}
	}

	// Helper function to send SSE data
	function sendSSEData(res, data, eventType = 'message') {
		const DEBUG = process.env.DEBUG === '1';
		try {
			if (DEBUG) {
				console.log(`[DEBUG] Sending SSE data, event type: ${eventType}`);
			}
			res.write(`event: ${eventType}\n`);
			res.write(`data: ${JSON.stringify(data)}\n\n`);
			if (DEBUG) {
				console.log(`[DEBUG] SSE data sent successfully for event: ${eventType}`);
				console.log(`[DEBUG] SSE data content: ${JSON.stringify(data).substring(0, 200)}${JSON.stringify(data).length > 200 ? '...' : ''}`);
			}
		} catch (error) {
			console.error(`[DEBUG] Error sending SSE data:`, error);
		}
	}

	// Map to store active chat instances by session ID
	const activeChatInstances = new Map();

	const server = createServer(async (req, res) => {
		// Apply authentication middleware to all requests first
		const processRequest = (routeHandler) => {
			// First apply authentication middleware
			authMiddleware(req, res, () => {
				// Then process the route if authentication passes
				routeHandler(req, res);
			});
		};

		// Define route handlers
		const routes = {
			// Static file routes
			'GET /logo.png': (req, res) => {
				const logoPath = join(__dirname, 'logo.png');
				if (existsSync(logoPath)) {
					res.writeHead(200, { 'Content-Type': 'image/png' });
					const logoData = readFileSync(logoPath);
					res.end(logoData);
				} else {
					res.writeHead(404, { 'Content-Type': 'text/plain' });
					res.end('Logo not found');
				}
			},
			// UI Routes
			'GET /': (req, res) => {
				res.writeHead(200, { 'Content-Type': 'text/html' });
				const html = readFileSync(join(__dirname, 'index.html'), 'utf8');

				// If we're in no API keys mode, add a flag to the HTML
				if (noApiKeysMode) {
					const modifiedHtml = html.replace('<body>', '<body data-no-api-keys="true">');
					res.end(modifiedHtml);
				} else {
					res.end(html);
				}
			},

			'GET /folders': (req, res) => {
				// Get the current working directory
				const currentWorkingDir = process.cwd();

				// Use the allowed folders if available, or default to the current working directory
				const folders = probeChat && probeChat.allowedFolders && probeChat.allowedFolders.length > 0
					? probeChat.allowedFolders
					: [currentWorkingDir];

				console.log(`Current working directory: ${currentWorkingDir}`);
				console.log(`Returning folders: ${JSON.stringify(folders)}`);

				res.writeHead(200, { 'Content-Type': 'application/json' });
				res.end(JSON.stringify({
					folders: folders,
					currentDir: currentWorkingDir,
					noApiKeysMode: noApiKeysMode
				}));
			},

			'GET /openapi.yaml': (req, res) => {
				const yamlPath = join(__dirname, 'openapi.yaml');
				if (existsSync(yamlPath)) {
					res.writeHead(200, { 'Content-Type': 'text/yaml' });
					const yaml = readFileSync(yamlPath, 'utf8');
					res.end(yaml);
				} else {
					res.writeHead(404, { 'Content-Type': 'text/plain' });
					res.end('OpenAPI specification not found');
				}
			},

			// SSE endpoint for tool calls - no authentication for easier testing
			'GET /api/tool-events': (req, res) => {
				// Parse session ID from query parameter
				let sessionId;
				try {
					const url = new URL(req.url, `http://${req.headers.host}`);
					sessionId = url.searchParams.get('sessionId');
					const DEBUG = process.env.DEBUG === '1';
					if (DEBUG) {
						console.log(`[DEBUG] Parsed URL: ${url.toString()}, sessionId: ${sessionId}`);
					}
				} catch (error) {
					const DEBUG = process.env.DEBUG === '1';
					if (DEBUG) {
						console.error(`[DEBUG] Error parsing URL: ${error.message}`);
					}
					// Fallback to manual parsing
					const queryString = req.url.split('?')[1] || '';
					const params = new URLSearchParams(queryString);
					sessionId = params.get('sessionId');
					if (DEBUG) {
						console.log(`[DEBUG] Manually parsed sessionId: ${sessionId}`);
					}
				}

				if (!sessionId) {
					if (process.env.DEBUG === '1') {
						console.error(`[DEBUG] No sessionId found in request URL: ${req.url}`);
					}
					res.writeHead(400, { 'Content-Type': 'application/json' });
					res.end(JSON.stringify({ error: 'Missing sessionId parameter' }));
					return;
				}

				const DEBUG = process.env.DEBUG === '1';
				if (DEBUG) {
					console.log(`[DEBUG] Setting up SSE connection for session: ${sessionId}`);
				}

				// Set headers for SSE
				res.writeHead(200, {
					'Content-Type': 'text/event-stream',
					'Cache-Control': 'no-cache',
					'Connection': 'keep-alive',
					'Access-Control-Allow-Origin': '*'
				});

				if (DEBUG) {
					console.log(`[DEBUG] SSE headers set for session: ${sessionId}`);
					console.log(`[DEBUG] Headers:`, {
						'Content-Type': 'text/event-stream',
						'Cache-Control': 'no-cache',
						'Connection': 'keep-alive',
						'Access-Control-Allow-Origin': '*'
					});
				}

				// Send initial connection established event
				const connectionData = {
					type: 'connection',
					message: 'Connection established',
					sessionId,
					timestamp: new Date().toISOString()
				};

				if (DEBUG) {
					console.log(`[DEBUG] Sending initial connection event for session: ${sessionId}`);
					console.log(`[DEBUG] Connection data:`, connectionData);
				}

				sendSSEData(res, connectionData, 'connection');

				// Send a test event to verify the connection is working
				setTimeout(() => {
					const DEBUG = process.env.DEBUG === '1';

					if (DEBUG) {
						console.log(`[DEBUG] Sending test event to session: ${sessionId}`);
					}

					const testData = {
						type: 'test',
						message: 'SSE connection test event',
						timestamp: new Date().toISOString(),
						sessionId,
						status: 'active',
						connectionInfo: {
							clientCount: sseClients.size,
							serverTime: new Date().toISOString(),
							testId: Math.random().toString(36).substring(2, 15)
						}
					};

					if (DEBUG) {
						console.log(`[DEBUG] Test event data:`, testData);
					}

					sendSSEData(res, testData, 'test');

					// Send a second test event after a short delay to verify continuous connection
					setTimeout(() => {
						if (DEBUG) {
							console.log(`[DEBUG] Sending follow-up test event to session: ${sessionId}`);
						}

						const followUpTestData = {
							type: 'test',
							message: 'SSE connection follow-up test event',
							timestamp: new Date().toISOString(),
							sessionId,
							status: 'confirmed',
							sequence: 2
						};

						sendSSEData(res, followUpTestData, 'test');
					}, 2000);
				}, 1000);

				// Function to handle tool call events for this session
				const handleToolCall = (toolCall) => {
					const DEBUG = process.env.DEBUG === '1';

					if (DEBUG) {
						console.log(`[DEBUG] Handling tool call for session ${sessionId}:`);
						console.log(`[DEBUG] Tool call name: ${toolCall.name}`);
						console.log(`[DEBUG] Tool call timestamp: ${toolCall.timestamp}`);
						console.log(`[DEBUG] Tool call args:`, toolCall.args);

						// Only log a preview of the result to avoid flooding the console
						if (toolCall.resultPreview) {
							const preview = toolCall.resultPreview.substring(0, 100) +
								(toolCall.resultPreview.length > 100 ? '... (truncated)' : '');
							console.log(`[DEBUG] Tool call result preview: ${preview}`);
						}
					}

					// Add a flag to indicate this is being sent via SSE
					const enhancedToolCall = {
						...toolCall,
						_via_sse: true,
						_sent_at: new Date().toISOString()
					};

					// Send the tool call data via SSE
					sendSSEData(res, enhancedToolCall, 'toolCall');

					if (DEBUG) {
						console.log(`[DEBUG] Tool call event sent via SSE for session ${sessionId}`);
					}
				};

				// Register event listener for this session
				const eventName = `toolCall:${sessionId}`;
				if (DEBUG) {
					console.log(`[DEBUG] Registering event listener for: ${eventName}`);
				}

				// Remove any existing listeners for this session to avoid duplicates
				toolCallEmitter.removeAllListeners(eventName);

				// Add the new listener
				toolCallEmitter.on(eventName, handleToolCall);
				if (DEBUG) {
					console.log(`[DEBUG] Registered event listener for session ${sessionId}`);
				}

				// Log the number of listeners
				if (process.env.DEBUG === '1') {
					const listenerCount = toolCallEmitter.listenerCount(eventName);
					console.log(`[DEBUG] Current listener count for ${eventName}: ${listenerCount}`);
				}

				// Add client to the map
				sseClients.set(sessionId, res);
				if (DEBUG) {
					console.log(`[DEBUG] Added SSE client for session ${sessionId}, total clients: ${sseClients.size}`);
				}

				// Handle client disconnect
				req.on('close', () => {
					if (DEBUG) {
						console.log(`[DEBUG] SSE client disconnecting: ${sessionId}`);
					}
					toolCallEmitter.removeListener(eventName, handleToolCall);
					sseClients.delete(sessionId);
					if (DEBUG) {
						console.log(`[DEBUG] SSE client disconnected: ${sessionId}, remaining clients: ${sseClients.size}`);
					}
				});
			},

			// Cancellation endpoint
			'POST /cancel-request': async (req, res) => {
				let body = '';
				req.on('data', chunk => body += chunk);
				req.on('end', async () => {
					try {
						const { sessionId } = JSON.parse(body);

						if (!sessionId) {
							res.writeHead(400, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({ error: 'Missing required parameter: sessionId' }));
							return;
						}

						const DEBUG = process.env.DEBUG === '1';
						if (DEBUG) {
							console.log(`\n[DEBUG] ===== Cancel Request =====`);
							console.log(`[DEBUG] Session ID: ${sessionId}`);
						}

						// Cancel any active tool executions for this session
						const toolExecutionsCancelled = cancelToolExecutions(sessionId);

						// Cancel the request in the request tracker
						const requestCancelled = cancelRequest(sessionId);

						// Get the chat instance for this session
						const chatInstance = activeChatInstances.get(sessionId);
						let chatInstanceAborted = false;

						if (chatInstance) {
							// Signal to the chat instance to abort
							if (typeof chatInstance.abort === 'function') {
								try {
									chatInstance.abort();
									chatInstanceAborted = true;
									if (DEBUG) {
										console.log(`[DEBUG] Aborted chat instance for session: ${sessionId}`);
									}
								} catch (error) {
									console.error(`Error aborting chat instance for session ${sessionId}:`, error);
								}
							}

							// Remove the chat instance
							activeChatInstances.delete(sessionId);
						}

						// Log the cancellation status
						console.log(`Cancellation status for session ${sessionId}:`);
						console.log(`- Tool executions cancelled: ${toolExecutionsCancelled}`);
						console.log(`- Request cancelled: ${requestCancelled}`);
						console.log(`- Chat instance aborted: ${chatInstanceAborted}`);

						res.writeHead(200, { 'Content-Type': 'application/json' });
						res.end(JSON.stringify({
							success: true,
							message: 'Cancellation request processed',
							details: {
								toolExecutionsCancelled,
								requestCancelled,
								chatInstanceAborted
							},
							timestamp: new Date().toISOString()
						}));
					} catch (error) {
						console.error('Error parsing request body:', error);
						res.writeHead(400, { 'Content-Type': 'application/json' });
						res.end(JSON.stringify({ error: 'Invalid JSON in request body' }));
					}
				});
			},

			// API Routes
			'POST /api/search': async (req, res) => {
				let body = '';
				req.on('data', chunk => body += chunk);
				req.on('end', async () => {
					try {
						const { keywords, folder, exact, allow_tests } = JSON.parse(body);

						if (!keywords) {
							res.writeHead(400, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({ error: 'Missing required parameter: keywords' }));
							return;
						}

						const DEBUG = process.env.DEBUG === '1';
						if (DEBUG) {
							console.log(`\n[DEBUG] ===== API Search Request =====`);
							console.log(`[DEBUG] Keywords: "${keywords}"`);
							console.log(`[DEBUG] Folder: "${folder || 'default'}"`);
							console.log(`[DEBUG] Exact match: ${exact ? 'yes' : 'no'}`);
							console.log(`[DEBUG] Allow tests: ${allow_tests ? 'yes' : 'no'}`);
						}

						try {
							// Get session ID from request if available
							const requestSessionId = JSON.parse(body).sessionId;
							const sessionId = requestSessionId || probeChat.getSessionId();
							if (DEBUG) {
								console.log(`[DEBUG] Using session ID for direct tool call: ${sessionId}`);
							}

							// Execute the probe tool directly
							const result = await probeTool.execute({
								keywords,
								folder: folder || (probeChat.allowedFolders && probeChat.allowedFolders.length > 0 ? probeChat.allowedFolders[0] : '.'),
								exact: exact || false,
								allow_tests: allow_tests || false
							});

							// Emit tool call event
							const toolCallData = {
								timestamp: new Date().toISOString(),
								name: 'searchCode',
								args: {
									keywords,
									folder: folder || '.',
									exact: exact || false,
									allow_tests: allow_tests || false
								},
								resultPreview: JSON.stringify(result).substring(0, 200) + '... (truncated)'
							};

							if (DEBUG) {
								console.log(`[DEBUG] Emitting direct tool call event for session ${sessionId}`);
							}
							// Add a unique ID to the tool call data to help with deduplication
							toolCallData.id = `${toolCallData.name}-${Date.now()}-${Math.random().toString(36).substring(2, 7)}`;
							toolCallEmitter.emit(`toolCall:${sessionId}`, toolCallData);

							res.writeHead(200, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify(result));
						} catch (error) {
							console.error('Error executing probe command:', error);

							// Determine the appropriate status code and error message
							let statusCode = 500;
							let errorMessage = 'Error executing probe command';

							if (error.code === 'ENOENT') {
								statusCode = 404;
								errorMessage = 'Folder not found or not accessible';
							} else if (error.code === 'EACCES') {
								statusCode = 403;
								errorMessage = 'Permission denied to access folder';
							} else if (error.message && error.message.includes('Invalid folder')) {
								statusCode = 400;
								errorMessage = 'Invalid folder specified';
							} else if (error.message && error.message.includes('timeout')) {
								statusCode = 504;
								errorMessage = 'Search operation timed out';
							}

							res.writeHead(statusCode, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({
								error: errorMessage,
								message: error.message,
								status: statusCode
							}));
						}
					} catch (error) {
						console.error('Error parsing request body:', error);
						res.writeHead(400, { 'Content-Type': 'application/json' });
						res.end(JSON.stringify({ error: 'Invalid JSON in request body' }));
					}
				});
			},

			'POST /api/query': async (req, res) => {
				let body = '';
				req.on('data', chunk => body += chunk);
				req.on('end', async () => {
					try {
						const { pattern, path, language, allow_tests } = JSON.parse(body);

						if (!pattern) {
							res.writeHead(400, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({ error: 'Missing required parameter: pattern' }));
							return;
						}

						const DEBUG = process.env.DEBUG === '1';
						if (DEBUG) {
							console.log(`\n[DEBUG] ===== API Query Request =====`);
							console.log(`[DEBUG] Pattern: "${pattern}"`);
							console.log(`[DEBUG] Path: "${path || 'default'}"`);
							console.log(`[DEBUG] Language: "${language || 'default'}"`);
							console.log(`[DEBUG] Allow tests: ${allow_tests ? 'yes' : 'no'}`);
						}

						try {
							// Get session ID from request if available
							const requestSessionId = JSON.parse(body).sessionId;
							const sessionId = requestSessionId || probeChat.getSessionId();
							if (DEBUG) {
								console.log(`[DEBUG] Using session ID for direct tool call: ${sessionId}`);
							}

							// Execute the query tool
							const result = await queryToolInstance.execute({
								pattern,
								path: path || (probeChat.allowedFolders && probeChat.allowedFolders.length > 0 ? probeChat.allowedFolders[0] : '.'),
								language: language || undefined,
								allow_tests: allow_tests || false
							});

							// Emit tool call event
							const toolCallData = {
								timestamp: new Date().toISOString(),
								name: 'queryCode',
								args: {
									pattern,
									path: path || '.',
									language: language || undefined,
									allow_tests: allow_tests || false
								},
								resultPreview: JSON.stringify(result).substring(0, 200) + '... (truncated)'
							};

							if (DEBUG) {
								console.log(`[DEBUG] Emitting direct tool call event for session ${sessionId}`);
							}
							// Add a unique ID to the tool call data to help with deduplication
							toolCallData.id = `${toolCallData.name}-${Date.now()}-${Math.random().toString(36).substring(2, 7)}`;
							toolCallEmitter.emit(`toolCall:${sessionId}`, toolCallData);

							res.writeHead(200, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({
								results: result,
								timestamp: new Date().toISOString()
							}));
						} catch (error) {
							console.error('Error executing query command:', error);

							// Determine the appropriate status code and error message
							let statusCode = 500;
							let errorMessage = 'Error executing query command';

							if (error.code === 'ENOENT') {
								statusCode = 404;
								errorMessage = 'Folder not found or not accessible';
							} else if (error.code === 'EACCES') {
								statusCode = 403;
								errorMessage = 'Permission denied to access folder';
							} else if (error.message && error.message.includes('Invalid folder')) {
								statusCode = 400;
								errorMessage = 'Invalid folder specified';
							} else if (error.message && error.message.includes('timeout')) {
								statusCode = 504;
								errorMessage = 'Search operation timed out';
							}

							res.writeHead(statusCode, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({
								error: errorMessage,
								message: error.message,
								status: statusCode
							}));
						}
					} catch (error) {
						console.error('Error parsing request body:', error);
						res.writeHead(400, { 'Content-Type': 'application/json' });
						res.end(JSON.stringify({ error: 'Invalid JSON in request body' }));
					}
				});
			},

			'POST /api/extract': async (req, res) => {
				let body = '';
				req.on('data', chunk => body += chunk);
				req.on('end', async () => {
					try {
						const { file_path, line, end_line, allow_tests, context_lines, format } = JSON.parse(body);

						if (!file_path) {
							res.writeHead(400, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({ error: 'Missing required parameter: file_path' }));
							return;
						}

						const DEBUG = process.env.DEBUG === '1';
						if (DEBUG) {
							console.log(`\n[DEBUG] ===== API Extract Request =====`);
							console.log(`[DEBUG] File path: "${file_path}"`);
							console.log(`[DEBUG] Line: ${line || 'not specified'}`);
							console.log(`[DEBUG] End line: ${end_line || 'not specified'}`);
							console.log(`[DEBUG] Allow tests: ${allow_tests ? 'yes' : 'no'}`);
							console.log(`[DEBUG] Context lines: ${context_lines || 'default'}`);
							console.log(`[DEBUG] Format: ${format || 'default'}`);
						}

						try {
							// Get session ID from request if available
							const requestSessionId = JSON.parse(body).sessionId;
							const sessionId = requestSessionId || probeChat.getSessionId();
							if (DEBUG) {
								console.log(`[DEBUG] Using session ID for direct tool call: ${sessionId}`);
							}

							// Execute the extract tool
							const result = await extractToolInstance.execute({
								file_path,
								line,
								end_line,
								allow_tests: allow_tests || false,
								context_lines: context_lines || 10,
								format: format || 'plain'
							});

							// Emit tool call event
							const toolCallData = {
								timestamp: new Date().toISOString(),
								name: 'extractCode',
								args: {
									file_path,
									line,
									end_line,
									allow_tests: allow_tests || false,
									context_lines: context_lines || 10,
									format: format || 'plain'
								},
								resultPreview: JSON.stringify(result).substring(0, 200) + '... (truncated)'
							};

							if (DEBUG) {
								console.log(`[DEBUG] Emitting direct tool call event for session ${sessionId}`);
							}
							// Add a unique ID to the tool call data to help with deduplication
							toolCallData.id = `${toolCallData.name}-${Date.now()}-${Math.random().toString(36).substring(2, 7)}`;
							toolCallEmitter.emit(`toolCall:${sessionId}`, toolCallData);

							res.writeHead(200, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({
								results: result,
								timestamp: new Date().toISOString()
							}));
						} catch (error) {
							console.error('Error executing extract command:', error);

							// Determine the appropriate status code and error message
							let statusCode = 500;
							let errorMessage = 'Error executing extract command';

							if (error.code === 'ENOENT') {
								statusCode = 404;
								errorMessage = 'File not found or not accessible';
							} else if (error.code === 'EACCES') {
								statusCode = 403;
								errorMessage = 'Permission denied to access file';
							} else if (error.message && error.message.includes('Invalid file')) {
								statusCode = 400;
								errorMessage = 'Invalid file specified';
							} else if (error.message && error.message.includes('timeout')) {
								statusCode = 504;
								errorMessage = 'Extract operation timed out';
							}

							res.writeHead(statusCode, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({
								error: errorMessage,
								message: error.message,
								status: statusCode
							}));
						}
					} catch (error) {
						console.error('Error parsing request body:', error);
						res.writeHead(400, { 'Content-Type': 'application/json' });
						res.end(JSON.stringify({ error: 'Invalid JSON in request body' }));
					}
				});
			},

			'POST /api/chat': async (req, res) => {
				let body = '';
				req.on('data', chunk => body += chunk);
				req.on('end', async () => {
					try {
						const { message, stream, sessionId, apiProvider, apiKey, apiUrl } = JSON.parse(body);

						if (!message) {
							res.writeHead(400, { 'Content-Type': 'application/json' });
							res.end(JSON.stringify({ error: 'Missing required parameter: message' }));
							return;
						}

						// Use the provided session ID or the default one
						const chatSessionId = sessionId || probeChat.getSessionId();

						// Create a temporary environment for this request if API key is provided
						let tempProbeChat = probeChat;
						if (apiKey) {
							const DEBUG = process.env.DEBUG === '1';
							if (DEBUG) {
								console.log(`[DEBUG] Using API key from request for provider: ${apiProvider}`);
							}

							// Create a new ProbeChat instance with the provided API key
							try {
								// Set temporary environment variables
								const originalEnv = {
									ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY,
									OPENAI_API_KEY: process.env.OPENAI_API_KEY,
									GOOGLE_API_KEY: process.env.GOOGLE_API_KEY,
									ANTHROPIC_API_URL: process.env.ANTHROPIC_API_URL,
									OPENAI_API_URL: process.env.OPENAI_API_URL,
									GOOGLE_API_URL: process.env.GOOGLE_API_URL,
									FORCE_PROVIDER: process.env.FORCE_PROVIDER
								};

								// Clear all API keys first
								process.env.ANTHROPIC_API_KEY = '';
								process.env.OPENAI_API_KEY = '';
								process.env.GOOGLE_API_KEY = '';

								// Set the provided API key
								if (apiProvider === 'anthropic') {
									process.env.ANTHROPIC_API_KEY = apiKey;
									if (apiUrl) process.env.ANTHROPIC_API_URL = apiUrl;
									process.env.FORCE_PROVIDER = 'anthropic';
								} else if (apiProvider === 'openai') {
									process.env.OPENAI_API_KEY = apiKey;
									if (apiUrl) process.env.OPENAI_API_URL = apiUrl;
									process.env.FORCE_PROVIDER = 'openai';
								} else if (apiProvider === 'google') {
									process.env.GOOGLE_API_KEY = apiKey;
									if (apiUrl) process.env.GOOGLE_API_URL = apiUrl;
									process.env.FORCE_PROVIDER = 'google';
								}

								// Create a new ProbeChat instance with the provided API key
								tempProbeChat = new ProbeChat({
									sessionId: chatSessionId
								});

								// Restore original environment variables after creating the instance
								process.env.ANTHROPIC_API_KEY = originalEnv.ANTHROPIC_API_KEY;
								process.env.OPENAI_API_KEY = originalEnv.OPENAI_API_KEY;
								process.env.GOOGLE_API_KEY = originalEnv.GOOGLE_API_KEY;
								process.env.ANTHROPIC_API_URL = originalEnv.ANTHROPIC_API_URL;
								process.env.OPENAI_API_URL = originalEnv.OPENAI_API_URL;
								process.env.GOOGLE_API_URL = originalEnv.GOOGLE_API_URL;
								process.env.FORCE_PROVIDER = originalEnv.FORCE_PROVIDER;

								if (DEBUG) {
									console.log(`[DEBUG] Created temporary ProbeChat instance with ${apiProvider} API key`);
								}
							} catch (error) {
								console.error('Error creating temporary ProbeChat instance:', error);
								// Fall back to the original instance
								tempProbeChat = probeChat;
							}
						}

						// Handle streaming vs non-streaming response
						const shouldStream = stream !== false; // Default to streaming

						if (!shouldStream) {
							// Non-streaming response (complete response as JSON)
							if (tempProbeChat !== probeChat) {
								// Use the temporary instance with the provided API key
								try {
									const responseText = await tempProbeChat.chat(message, chatSessionId);
									const tokenUsage = tempProbeChat.getTokenUsage();

									res.writeHead(200, { 'Content-Type': 'application/json' });
									res.end(JSON.stringify({
										response: responseText,
										tokenUsage: tokenUsage,
										timestamp: new Date().toISOString()
									}));
								} catch (error) {
									console.error('Error generating response with provided API key:', error);
									res.writeHead(500, { 'Content-Type': 'application/json' });
									res.end(JSON.stringify({
										error: 'Error generating response with provided API key',
										message: error.message,
										status: 500
									}));
								}
							} else {
								// Use the original instance
								await handleNonStreamingChatRequest(req, res, message, chatSessionId);
							}
						} else {
							// Streaming response (chunks of text)
							if (tempProbeChat !== probeChat) {
								// Use the temporary instance with the provided API key
								try {
									res.writeHead(200, {
										'Content-Type': 'text/plain',
										'Transfer-Encoding': 'chunked',
										'Cache-Control': 'no-cache',
										'Connection': 'keep-alive'
									});

									const responseText = await tempProbeChat.chat(message, chatSessionId);
									res.write(responseText);
									res.end();
								} catch (error) {
									console.error('Error streaming response with provided API key:', error);
									res.writeHead(500, { 'Content-Type': 'text/plain' });
									res.end(`Error: Error generating response with provided API key - ${error.message}`);
								}
							} else {
								// Use the original instance
								await handleStreamingChatRequest(req, res, message, chatSessionId);
							}
						}
					} catch (error) {
						console.error('Error parsing request body:', error);
						res.writeHead(400, { 'Content-Type': 'application/json' });
						res.end(JSON.stringify({ error: 'Invalid JSON in request body' }));
					}
				});
			},

			'POST /chat': (req, res) => {
				let body = '';
				req.on('data', chunk => body += chunk);
				req.on('end', async () => {
					try {
						const requestData = JSON.parse(body);
						const { message, sessionId, apiProvider, apiKey, apiUrl } = requestData;

						const DEBUG = process.env.DEBUG === '1';
						// Debug logs moved to conditional block below

						if (DEBUG) {
							console.log(`\n[DEBUG] ===== Chat Request =====`);
							console.log(`[DEBUG] Full request data:`, requestData);
							console.log(`[DEBUG] User message: "${message}"`);
							console.log(`[DEBUG] Session ID: ${sessionId || 'not provided'}`);
							if (sessionId) {
								console.log(`[DEBUG] Session ID: ${sessionId}`);
							}
							if (apiKey) {
								console.log(`[DEBUG] API key provided for provider: ${apiProvider}`);
							}
						}

						res.writeHead(200, {
							'Content-Type': 'text/plain',
							'Transfer-Encoding': 'chunked',
							'Cache-Control': 'no-cache',
							'Connection': 'keep-alive'
						});

						// Use the provided session ID or the default one
						const chatSessionId = sessionId || probeChat.getSessionId();

						// Register this request as active
						registerRequest(chatSessionId, {
							abort: () => {
								// This will be called when the request is cancelled
								console.log(`Aborting request for session: ${chatSessionId}`);
								// We'll add abort functionality to the ProbeChat class
							}
						});
						if (DEBUG) {
							console.log(`[DEBUG] Using session ID for chat: ${chatSessionId}`);
						}

						// Create a temporary environment for this request if API key is provided
						let tempProbeChat = probeChat;
						if (apiKey) {
							if (DEBUG) {
								console.log(`[DEBUG] Using API key from request for provider: ${apiProvider}`);
							}

							// Create a new ProbeChat instance with the provided API key
							try {
								// Set temporary environment variables
								const originalEnv = {
									ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY,
									OPENAI_API_KEY: process.env.OPENAI_API_KEY,
									GOOGLE_API_KEY: process.env.GOOGLE_API_KEY,
									ANTHROPIC_API_URL: process.env.ANTHROPIC_API_URL,
									OPENAI_API_URL: process.env.OPENAI_API_URL,
									GOOGLE_API_URL: process.env.GOOGLE_API_URL,
									FORCE_PROVIDER: process.env.FORCE_PROVIDER
								};

								// Clear all API keys first
								process.env.ANTHROPIC_API_KEY = '';
								process.env.OPENAI_API_KEY = '';
								process.env.GOOGLE_API_KEY = '';

								// Set the provided API key
								if (apiProvider === 'anthropic') {
									process.env.ANTHROPIC_API_KEY = apiKey;
									if (apiUrl) process.env.ANTHROPIC_API_URL = apiUrl;
									process.env.FORCE_PROVIDER = 'anthropic';
								} else if (apiProvider === 'openai') {
									process.env.OPENAI_API_KEY = apiKey;
									if (apiUrl) process.env.OPENAI_API_URL = apiUrl;
									process.env.FORCE_PROVIDER = 'openai';
								} else if (apiProvider === 'google') {
									process.env.GOOGLE_API_KEY = apiKey;
									if (apiUrl) process.env.GOOGLE_API_URL = apiUrl;
									process.env.FORCE_PROVIDER = 'google';
								}

								// Create a new ProbeChat instance with the provided API key
								tempProbeChat = new ProbeChat({
									sessionId: chatSessionId
								});

								// Restore original environment variables after creating the instance
								process.env.ANTHROPIC_API_KEY = originalEnv.ANTHROPIC_API_KEY;
								process.env.OPENAI_API_KEY = originalEnv.OPENAI_API_KEY;
								process.env.GOOGLE_API_KEY = originalEnv.GOOGLE_API_KEY;
								process.env.ANTHROPIC_API_URL = originalEnv.ANTHROPIC_API_URL;
								process.env.OPENAI_API_URL = originalEnv.OPENAI_API_URL;
								process.env.GOOGLE_API_URL = originalEnv.GOOGLE_API_URL;
								process.env.FORCE_PROVIDER = originalEnv.FORCE_PROVIDER;

								if (DEBUG) {
									console.log(`[DEBUG] Created temporary ProbeChat instance with ${apiProvider} API key`);
								}
							} catch (error) {
								console.error('Error creating temporary ProbeChat instance:', error);
								// Fall back to the original instance
								tempProbeChat = probeChat;
							}
						}

						// Store the chat instance for potential cancellation
						const chatInstance = tempProbeChat;
						chatInstance.abort = () => {
							console.log(`Aborting chat for session: ${chatSessionId}`);
							// The actual abort functionality will be added to ProbeChat
						};
						activeChatInstances.set(chatSessionId, chatInstance);

						try {
							// Use the appropriate ProbeChat instance to get a response
							const responseText = await tempProbeChat.chat(message, chatSessionId);

							// Write the response
							res.write(responseText);
							res.end();

							// Get token usage
							const tokenUsage = tempProbeChat.getTokenUsage();
							totalRequestTokens = tokenUsage.request;
							totalResponseTokens = tokenUsage.response;

							// Clear the request from active requests
							clearRequest(chatSessionId);
							activeChatInstances.delete(chatSessionId);
						} catch (error) {
							// If the error is due to cancellation, handle it gracefully
							if (error.message && error.message.includes('cancelled')) {
								console.log(`Chat request was cancelled for session: ${chatSessionId}`);
								res.write('\n\n*Request was cancelled by the user.*');
								res.end();
							} else {
								// Re-throw other errors to be caught by the outer catch block
								throw error;
							}

							// Clear the request from active requests
							clearRequest(chatSessionId);
							activeChatInstances.delete(chatSessionId);
						}

						console.log('Finished streaming response');

						// Clear any tool execution data
						clearToolExecutionData(chatSessionId);
					} catch (error) {
						console.error('Error processing chat request:', error);

						// Determine the appropriate status code and error message
						let statusCode = 500;
						let errorMessage = 'Internal Server Error';

						if (error instanceof SyntaxError) {
							statusCode = 400;
							errorMessage = 'Invalid JSON in request body';
						} else if (error.code === 'EACCES') {
							statusCode = 403;
							errorMessage = 'Permission denied';
						} else if (error.code === 'ENOENT') {
							statusCode = 404;
							errorMessage = 'Resource not found';
						}

						res.writeHead(statusCode, { 'Content-Type': 'text/plain' });
						res.end(`${errorMessage}: ${error.message}`);
					}
				});
			}
		};

		// Route handling logic
		const method = req.method;
		const url = req.url;
		const routeKey = `${method} ${url}`;
		// Check if we have an exact route match
		if (routes[routeKey]) {
			// Skip authentication for public routes
			if (routeKey === 'GET /openapi.yaml' || routeKey === 'GET /api/tool-events') {
				return routes[routeKey](req, res);
			}
			// Apply authentication for protected routes
			return processRequest(routes[routeKey]);
		}
		// Check for partial matches (e.g., /api/chat?param=value should match 'POST /api/chat')
		const baseUrl = url.split('?')[0];
		const baseRouteKey = `${method} ${baseUrl}`;

		if (routes[baseRouteKey]) {
			// Skip authentication for public routes
			if (baseRouteKey === 'GET /openapi.yaml' || baseRouteKey === 'GET /api/tool-events') {
				return routes[baseRouteKey](req, res);
			}
			// Apply authentication for protected routes
			return processRequest(routes[baseRouteKey]);
		}

		// No route match, return 404
		res.writeHead(404, { 'Content-Type': 'text/plain' });
		res.end('Not Found');
	});

	// Start the server
	const PORT = process.env.PORT || 8080;
	server.listen(PORT, () => {
		console.log(`Probe Web Interface v${version}`);
		console.log(`Server running on http://localhost:${PORT}`);
		console.log(`Environment: ${process.env.NODE_ENV || 'development'}`);

		if (noApiKeysMode) {
			console.log('Running in NO API KEYS MODE - setup instructions will be shown to users');
		} else {
			console.log('Probe tool is available for AI to use');
			console.log(`Session ID: ${probeChat.getSessionId()}`);
		}
	});
}