import 'dotenv/config';
import { createServer } from 'http';
// import { streamText } from 'ai'; // streamText might not be suitable for the loop logic directly
import { readFileSync, existsSync } from 'fs';
import { resolve, dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { randomUUID } from 'crypto';
import { ProbeChat } from './probeChat.js';
import { TokenUsageDisplay } from './tokenUsageDisplay.js';
import { authMiddleware, withAuth } from './auth.js';
import {
	// probeTool, // This is the compatibility layer, less critical now
	searchToolInstance, // Keep direct instances for API endpoints
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


// Map to store chat instances by session ID
const chatSessions = new Map();

/**
 * Retrieve or create a ProbeChat instance keyed by sessionId.
 */
function getOrCreateChat(sessionId, apiCredentials = null) {
	if (!sessionId) {
		// Safety fallback: generate a random ID if missing
		sessionId = randomUUID(); // Use crypto.randomUUID() if available/preferred
		console.warn(`[WARN] Missing sessionId, generated fallback: ${sessionId}`);
	}
	if (chatSessions.has(sessionId)) {
		return chatSessions.get(sessionId);
	}

	// Create options object with sessionId and API credentials if provided
	const options = { sessionId };
	if (apiCredentials) {
		options.apiProvider = apiCredentials.apiProvider;
		options.apiKey = apiCredentials.apiKey;
		options.apiUrl = apiCredentials.apiUrl;
	}

	const newChat = new ProbeChat(options);
	chatSessions.set(sessionId, newChat);
	if (process.env.DEBUG_CHAT === '1') {
		console.log(`[DEBUG] Created and stored new chat instance for session: ${sessionId}. Total sessions: ${chatSessions.size}`);
		if (apiCredentials && apiCredentials.apiKey) {
			console.log(`[DEBUG] Chat instance created with client-provided API credentials (provider: ${apiCredentials.apiProvider})`);
		}
	}
	return newChat;
}

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

	// Initialize a default ProbeChat instance for /folders endpoint? Or make folders static?
	// Let's make /folders rely on environment variables directly or a static config
	// to avoid needing a default chat instance just for that.
	const staticAllowedFolders = process.env.ALLOWED_FOLDERS
		? process.env.ALLOWED_FOLDERS.split(',').map(folder => folder.trim()).filter(Boolean)
		: [];


	let noApiKeysMode = !hasApiKeys;
	if (noApiKeysMode) {
		console.log('Running in No API Keys mode - will show setup instructions to users');
	} else {
		console.log('API keys detected. Chat functionality enabled.');
	}


	// Define the tools available for direct API calls (bypassing LLM loop)
	// Note: probeTool is the backward compatibility wrapper
	const directApiTools = {
		search: searchToolInstance,
		query: queryToolInstance,
		extract: extractToolInstance
	};


	// Helper function to send SSE data
	function sendSSEData(res, data, eventType = 'message') {
		const DEBUG = process.env.DEBUG_CHAT === '1';
		try {
			// Check if the response stream is still writable
			if (!res.writable || res.writableEnded) {
				if (DEBUG) console.log(`[DEBUG] SSE stream closed for event type ${eventType}, cannot send.`);
				return;
			}
			if (DEBUG) {
				// console.log(`[DEBUG] Sending SSE data, event type: ${eventType}`); // Can be noisy
			}
			res.write(`event: ${eventType}\n`);
			res.write(`data: ${JSON.stringify(data)}\n\n`);
			if (DEBUG) {
				// console.log(`[DEBUG] SSE data sent successfully for event: ${eventType}`);
				// const preview = JSON.stringify(data).substring(0, 100);
				// console.log(`[DEBUG] SSE data content preview: ${preview}...`);
			}
		} catch (error) {
			console.error(`[ERROR] Error sending SSE data:`, error);
			// Attempt to close the connection gracefully on error?
			try {
				if (res.writable && !res.writableEnded) res.end();
			} catch (closeError) {
				console.error(`[ERROR] Error closing SSE stream after send error:`, closeError);
			}
		}
	}

	// Map to store active chat instances by session ID for cancellation purposes
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
			// Handle OPTIONS requests for CORS preflight (Common)
			'OPTIONS /api/token-usage': (req, res) => handleOptions(res),
			'OPTIONS /chat': (req, res) => handleOptions(res),
			'OPTIONS /api/search': (req, res) => handleOptions(res),
			'OPTIONS /api/query': (req, res) => handleOptions(res),
			'OPTIONS /api/extract': (req, res) => handleOptions(res),
			'OPTIONS /cancel-request': (req, res) => handleOptions(res),
			'OPTIONS /folders': (req, res) => handleOptions(res), // Added for /folders


			// Token usage API endpoint
			'GET /api/token-usage': (req, res) => {
				const sessionId = getSessionIdFromUrl(req);
				if (!sessionId) return sendError(res, 400, 'Missing sessionId parameter');

				const chatInstance = chatSessions.get(sessionId);
				if (!chatInstance) return sendError(res, 404, 'Session not found');

				const DEBUG = process.env.DEBUG_CHAT === '1';

				// Update the tokenCounter's history with the chat history
				if (chatInstance.tokenCounter && typeof chatInstance.tokenCounter.updateHistory === 'function' &&
					chatInstance.history) {
					chatInstance.tokenCounter.updateHistory(chatInstance.history);
					if (DEBUG) {
						console.log(`[DEBUG] Updated tokenCounter history with ${chatInstance.history.length} messages for token usage request`);
					}
				}

				// Get raw token usage data from the chat instance
				const tokenUsage = chatInstance.getTokenUsage();

				if (DEBUG) {
					console.log(`[DEBUG] Token usage request - Context window size: ${tokenUsage.contextWindow}`);
					console.log(`[DEBUG] Token usage request - Cache metrics - Read: ${tokenUsage.current.cacheRead}, Write: ${tokenUsage.current.cacheWrite}`);
				}

				// Send the raw token usage data to the client
				// The client-side JavaScript will handle formatting
				sendJson(res, 200, tokenUsage);
			},
			// Static file routes
			'GET /logo.png': (req, res) => serveStatic(res, join(__dirname, 'logo.png'), 'image/png'),
			// UI Routes
			'GET /': (req, res) => {
				const htmlPath = join(__dirname, 'index.html');
				serveHtml(res, htmlPath, { 'data-no-api-keys': noApiKeysMode ? 'true' : 'false' });
			},

			'GET /folders': (req, res) => {
				const currentWorkingDir = process.cwd();
				// Use static config or environment variables directly
				const folders = staticAllowedFolders.length > 0 ? staticAllowedFolders : [currentWorkingDir];

				sendJson(res, 200, {
					folders: folders,
					currentDir: currentWorkingDir,
					noApiKeysMode: noApiKeysMode
				});
			},

			'GET /openapi.yaml': (req, res) => serveStatic(res, join(__dirname, 'openapi.yaml'), 'text/yaml'),


			// SSE endpoint for tool calls - NO AUTH for easier client implementation
			'GET /api/tool-events': (req, res) => {
				const DEBUG = process.env.DEBUG_CHAT === '1';
				const sessionId = getSessionIdFromUrl(req);
				if (!sessionId) {
					if (DEBUG) console.error(`[DEBUG] SSE: No sessionId found in URL: ${req.url}`);
					return sendError(res, 400, 'Missing sessionId parameter');
				}

				if (DEBUG) console.log(`[DEBUG] SSE: Setting up connection for session: ${sessionId}`);

				// Set headers for SSE
				res.writeHead(200, {
					'Content-Type': 'text/event-stream',
					'Cache-Control': 'no-cache',
					'Connection': 'keep-alive',
					'Access-Control-Allow-Origin': '*' // Allow all origins for SSE
				});
				if (DEBUG) console.log(`[DEBUG] SSE: Headers set for session: ${sessionId}`);

				// Send initial connection established event
				const connectionData = { type: 'connection', message: 'SSE Connection Established', sessionId, timestamp: new Date().toISOString() };
				sendSSEData(res, connectionData, 'connection');
				if (DEBUG) console.log(`[DEBUG] SSE: Sent connection event for session: ${sessionId}`);

				// Send a test event (optional, for debugging)
				// setTimeout(() => sendSSEData(res, { type: 'test', message: 'Test event', sessionId }, 'test'), 1000);

				// Function to handle tool call events for this session
				const handleToolCall = (toolCall) => {
					if (DEBUG) {
						// console.log(`[DEBUG] SSE: Handling tool call event for session ${sessionId}: ${toolCall.name}`); // Noisy
					}
					// Ensure data is serializable and add timestamp if missing
					const serializableCall = {
						...toolCall,
						timestamp: toolCall.timestamp || new Date().toISOString(),
						_sse_sent_at: new Date().toISOString()
					};
					sendSSEData(res, serializableCall, 'toolCall'); // Event type 'toolCall'
				};

				// Register event listener for this specific session
				const eventName = `toolCall:${sessionId}`;
				// Remove previous listener for this exact session ID if any (safety measure)
				const existingHandler = sseClients.get(sessionId)?.handler;
				if (existingHandler) {
					toolCallEmitter.removeListener(eventName, existingHandler);
				}

				toolCallEmitter.on(eventName, handleToolCall);
				if (DEBUG) console.log(`[DEBUG] SSE: Registered listener for ${eventName}`);

				// Store client and handler for cleanup
				sseClients.set(sessionId, { res, handler: handleToolCall });
				if (DEBUG) console.log(`[DEBUG] SSE: Client added for session ${sessionId}. Total clients: ${sseClients.size}`);

				// Handle client disconnect
				req.on('close', () => {
					if (DEBUG) console.log(`[DEBUG] SSE: Client disconnecting: ${sessionId}`);
					toolCallEmitter.removeListener(eventName, handleToolCall);
					sseClients.delete(sessionId);
					if (DEBUG) console.log(`[DEBUG] SSE: Client removed for session ${sessionId}. Remaining clients: ${sseClients.size}`);
				});
			},

			// Cancellation endpoint
			'POST /cancel-request': async (req, res) => {
				handlePostRequest(req, res, async (body) => {
					const { sessionId } = body;
					if (!sessionId) return sendError(res, 400, 'Missing required parameter: sessionId');

					const DEBUG = process.env.DEBUG_CHAT === '1';
					if (DEBUG) console.log(`\n[DEBUG] ===== Cancel Request for Session: ${sessionId} =====`);

					// 1. Cancel Tool Executions (via probeTool.js)
					const toolExecutionsCancelled = cancelToolExecutions(sessionId);

					// 2. Cancel Active Chat Request (via probeChat instance)
					const chatInstance = activeChatInstances.get(sessionId);
					let chatInstanceAborted = false;
					if (chatInstance && typeof chatInstance.abort === 'function') {
						try {
							chatInstance.abort(); // This sets chatInstance.cancelled = true and aborts controller
							chatInstanceAborted = true;
							if (DEBUG) console.log(`[DEBUG] Aborted chat instance processing for session: ${sessionId}`);
						} catch (error) {
							console.error(`Error aborting chat instance for session ${sessionId}:`, error);
						}
					} else {
						if (DEBUG) console.log(`[DEBUG] No active chat instance found in map for session ${sessionId} to abort.`);
					}

					// 3. Cancel the request tracking entry (via cancelRequest.js - might be redundant if chatInstance.abort works)
					const requestCancelled = cancelRequest(sessionId); // This calls the registered abort function

					// Clean up map entry (might be done in finally block of chat endpoint too)
					activeChatInstances.delete(sessionId);


					console.log(`Cancellation processed for session ${sessionId}: Tools=${toolExecutionsCancelled}, Chat=${chatInstanceAborted}, RequestTracking=${requestCancelled}`);

					sendJson(res, 200, {
						success: true,
						message: 'Cancellation request processed',
						details: { toolExecutionsCancelled, chatInstanceAborted, requestCancelled },
						timestamp: new Date().toISOString()
					});
				});
			},

			// --- Direct API Tool Endpoints (Bypass LLM Loop) ---
			'POST /api/search': async (req, res) => {
				handlePostRequest(req, res, async (body) => {
					const { query, path, allow_tests, maxResults, maxTokens, sessionId: reqSessionId } = body; // Renamed params
					if (!query) return sendError(res, 400, 'Missing required parameter: query');

					const sessionId = reqSessionId || randomUUID(); // Use provided or generate new for direct call
					const toolParams = { query, path, allow_tests, maxResults, maxTokens, sessionId };

					await executeDirectTool(res, directApiTools.search, 'search', toolParams, sessionId);
				});
			},
			'POST /api/query': async (req, res) => {
				handlePostRequest(req, res, async (body) => {
					const { pattern, path, language, allow_tests, sessionId: reqSessionId } = body;
					if (!pattern) return sendError(res, 400, 'Missing required parameter: pattern');

					const sessionId = reqSessionId || randomUUID();
					const toolParams = { pattern, path, language, allow_tests, sessionId };

					await executeDirectTool(res, directApiTools.query, 'query', toolParams, sessionId);
				});
			},
			'POST /api/extract': async (req, res) => {
				handlePostRequest(req, res, async (body) => {
					const { file_path, line, end_line, allow_tests, context_lines, format, input_content, sessionId: reqSessionId } = body;
					// file_path or input_content is required by the underlying tool implementation usually
					if (!file_path && !input_content) return sendError(res, 400, 'Missing required parameter: file_path or input_content');

					const sessionId = reqSessionId || randomUUID();
					const toolParams = { file_path, line, end_line, allow_tests, context_lines, format, input_content, sessionId };

					await executeDirectTool(res, directApiTools.extract, 'extract', toolParams, sessionId);
				});
			},

			// --- Main Chat Endpoint (Handles the Loop) ---
			'POST /chat': (req, res) => { // This is the route used by the frontend UI
				handlePostRequest(req, res, async (requestData) => {
					const {
						message,
						sessionId: reqSessionId,
						clearHistory,
						apiProvider,
						apiKey,
						apiUrl
					} = requestData;
					const DEBUG = process.env.DEBUG_CHAT === '1';

					if (DEBUG) {
						console.log(`\n[DEBUG] ===== UI Chat Request =====`);
						console.log(`[DEBUG] Request Data:`, { ...requestData, apiKey: requestData.apiKey ? '******' : undefined });
					}

					// --- Session and Instance Management ---
					const chatSessionId = reqSessionId || randomUUID(); // Ensure we always have a session ID
					if (!reqSessionId && DEBUG) console.log(`[DEBUG] No session ID from UI, generated: ${chatSessionId}`);
					else if (DEBUG) console.log(`[DEBUG] Using session ID from UI: ${chatSessionId}`);

					// Get or create the chat instance *without* API key overrides here.
					// API keys from request are ignored for existing sessions to preserve history consistency.
					// If a *new* session is created AND keys are provided, the ProbeChat constructor *should* handle them.
					// Extract API credentials from request if available
					const apiCredentials = apiKey ? { apiProvider, apiKey, apiUrl } : null;

					// Get or create chat instance with API credentials
					const chatInstance = getOrCreateChat(chatSessionId, apiCredentials);

					// Check if API keys are needed but missing
					if (chatInstance.noApiKeysMode) {
						console.warn(`[WARN] Chat request for session ${chatSessionId} cannot proceed: No API keys configured.`);
						return sendError(res, 503, 'Chat service unavailable: API key not configured on server.');
					}

					// Register this request as active for cancellation
					registerRequest(chatSessionId, { abort: () => chatInstance.abort() });
					if (DEBUG) console.log(`[DEBUG] Registered cancellable request for session: ${chatSessionId}`);
					activeChatInstances.set(chatSessionId, chatInstance); // Store for direct access during cancellation

					// --- Handle Clear History ---
					if (message === '__clear_history__' || clearHistory) {
						console.log(`Clearing chat history for session: ${chatSessionId}`);
						const newSessionId = chatInstance.clearHistory(); // clearHistory now returns the *new* session ID
						// Remove old session state
						clearRequest(chatSessionId);
						activeChatInstances.delete(chatSessionId);
						clearToolExecutionData(chatSessionId);
						chatSessions.delete(chatSessionId); // Remove old instance from map
						// We don't create the *new* instance here, it will be created on the *next* message request

						// Create a new empty token usage object for the cleared history
						const emptyTokenUsage = {
							contextWindow: 0,
							current: {
								request: 0,
								response: 0,
								total: 0,
								cacheRead: 0,
								cacheWrite: 0,
								cacheTotal: 0
							},
							total: {
								request: 0,
								response: 0,
								total: 0,
								cacheRead: 0,
								cacheWrite: 0,
								cacheTotal: 0
							}
						};

						sendJson(res, 200, {
							response: 'Chat history cleared',
							tokenUsage: emptyTokenUsage, // Include empty token usage data
							newSessionId: newSessionId, // Inform UI about the new ID
							timestamp: new Date().toISOString()
						});
						return; // Stop processing
					}

					// --- Execute Chat Loop (Non-Streaming Response) ---
					// The loop is inside chatInstance.chat now.
					// We expect the *final* result string back.
					try {
						// Pass API credentials to the chat method if provided
						const apiCredentials = apiKey ? { apiProvider, apiKey, apiUrl } : null;
						const result = await chatInstance.chat(message, chatSessionId, apiCredentials); // Pass session ID and API credentials

						// Check if cancelled *during* the chat call (ProbeChat throws error)
						// Error handled in catch block

						// Handle the new structured response format
						let responseText;
						let tokenUsage;

						if (result && typeof result === 'object' && 'response' in result) {
							// New format: { response: string, tokenUsage: object }
							responseText = result.response;
							tokenUsage = result.tokenUsage;

							if (process.env.DEBUG_CHAT === '1') {
								console.log(`[DEBUG] Received structured response with token usage data`);
								console.log(`[DEBUG] Context window size: ${tokenUsage.contextWindow}`);
								console.log(`[DEBUG] Cache metrics - Read: ${tokenUsage.current.cacheRead}, Write: ${tokenUsage.current.cacheWrite}`);
							}
						} else {
							// Legacy format: string response
							responseText = result;
							tokenUsage = chatInstance.getTokenUsage(); // Get token usage separately

							if (process.env.DEBUG_CHAT === '1') {
								console.log(`[DEBUG] Received legacy response format, fetched token usage separately`);
							}
						}

						// Create the response object with the response text and token usage data
						const responseObject = {
							response: responseText,
							tokenUsage: tokenUsage,
							sessionId: chatSessionId,
							timestamp: new Date().toISOString()
						};

						// Send the response with token usage in both the body and header
						sendJson(res, 200, responseObject, { 'X-Token-Usage': JSON.stringify(tokenUsage) });

						console.log(`Finished chat request for session: ${chatSessionId}`);

					} catch (error) {
						// Check if the error is actually a structured response with token usage
						let errorResponse = error;
						let tokenUsage;

						if (error && typeof error === 'object' && error.response && error.tokenUsage) {
							// This is a structured error response from probeChat
							errorResponse = error.response;
							tokenUsage = error.tokenUsage;

							if (process.env.DEBUG_CHAT === '1') {
								console.log(`[DEBUG] Received structured error response with token usage data`);
								console.log(`[DEBUG] Context window size: ${tokenUsage.contextWindow}`);
								console.log(`[DEBUG] Cache metrics - Read: ${tokenUsage.current.cacheRead}, Write: ${tokenUsage.current.cacheWrite}`);
							}
						} else {
							// Get token usage separately for regular errors

							// First update the tokenCounter's history with the chat history
							if (chatInstance.tokenCounter && typeof chatInstance.tokenCounter.updateHistory === 'function' &&
								chatInstance.history) {
								chatInstance.tokenCounter.updateHistory(chatInstance.history);
								if (DEBUG) {
									console.log(`[DEBUG] Updated tokenCounter history with ${chatInstance.history.length} messages for error case`);
								}
							}

							// Force recalculation of context window size
							if (chatInstance.tokenCounter && typeof chatInstance.tokenCounter.calculateContextSize === 'function') {
								chatInstance.tokenCounter.calculateContextSize(chatInstance.history);
								if (DEBUG) {
									console.log(`[DEBUG] Forced recalculation of context window size for error case`);
								}
							}

							// Get updated token usage after history update and recalculation
							tokenUsage = chatInstance.getTokenUsage();

							if (DEBUG) {
								console.log(`[DEBUG] Error case - Final context window size: ${tokenUsage.contextWindow}`);
								console.log(`[DEBUG] Error case - Cache metrics - Read: ${tokenUsage.current.cacheRead}, Write: ${tokenUsage.current.cacheWrite}`);
							}
						}

						// Handle errors, including cancellation
						if (errorResponse.message && errorResponse.message.includes('cancelled') ||
							(typeof errorResponse === 'string' && errorResponse.includes('cancelled'))) {
							console.log(`Chat request processing was cancelled for session: ${chatSessionId}`);
							// Send structured error response with token usage
							sendJson(res, 499, {
								error: 'Request cancelled by user',
								tokenUsage: tokenUsage,
								sessionId: chatSessionId,
								timestamp: new Date().toISOString()
							}); // 499 Client Closed Request
						} else {
							console.error(`Error processing chat for session ${chatSessionId}:`, error);
							// Send structured error response with token usage
							sendJson(res, 500, {
								error: `Chat processing error: ${typeof errorResponse === 'string' ? errorResponse : errorResponse.message || 'Unknown error'}`,
								tokenUsage: tokenUsage,
								sessionId: chatSessionId,
								timestamp: new Date().toISOString()
							});
						}
					} finally {
						// Cleanup regardless of success, error, or cancellation
						clearRequest(chatSessionId);
						activeChatInstances.delete(chatSessionId);
						// Don't clear tool execution data here, it might be needed if user retries
						// clearToolExecutionData(chatSessionId);
						if (DEBUG) console.log(`[DEBUG] Cleaned up active request tracking for session: ${chatSessionId}`);
					}
				}); // End handlePostRequest for /chat
			} // End /chat route
		}; // End routes object

		// --- Request Routing ---
		const parsedUrl = new URL(req.url, `http://${req.headers.host}`);
		const routeKey = `${req.method} ${parsedUrl.pathname}`;
		const handler = routes[routeKey];

		if (handler) {
			// Skip auth for specific public routes
			const publicRoutes = ['GET /openapi.yaml', 'GET /api/tool-events', 'GET /logo.png', 'GET /', 'GET /folders', 'OPTIONS']; // Add OPTIONS
			if (publicRoutes.includes(routeKey) || req.method === 'OPTIONS') {
				handler(req, res);
			} else {
				processRequest(handler); // Apply auth middleware
			}
		} else {
			// No route match, return 404
			sendError(res, 404, 'Not Found');
		}
	}); // End createServer

	// Start the server
	const PORT = process.env.PORT || 8080;
	server.listen(PORT, () => {
		console.log(`Probe Web Interface v${version}`);
		console.log(`Server running on http://localhost:${PORT}`);
		console.log(`Environment: ${process.env.NODE_ENV || 'development'}`);
		if (noApiKeysMode) {
			console.log('*** Running in NO API KEYS mode. Chat functionality disabled. ***');
		}
	});
}


// --- Helper Functions ---

function handleOptions(res) {
	res.writeHead(200, {
		'Access-Control-Allow-Origin': '*', // Or specific origin
		'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
		'Access-Control-Allow-Headers': 'Content-Type, Authorization, X-Session-ID', // Add any custom headers needed
		'Access-Control-Max-Age': '86400' // 24 hours
	});
	res.end();
}

function sendJson(res, statusCode, data, headers = {}) {
	if (res.headersSent) return;
	res.writeHead(statusCode, {
		'Content-Type': 'application/json',
		'Access-Control-Allow-Origin': '*', // Adjust as needed
		'Access-Control-Expose-Headers': 'X-Token-Usage', // Expose custom headers
		...headers
	});
	res.end(JSON.stringify(data));
}

function sendError(res, statusCode, message) {
	if (res.headersSent) return;
	console.error(`Sending error (${statusCode}): ${message}`);
	res.writeHead(statusCode, {
		'Content-Type': 'application/json',
		'Access-Control-Allow-Origin': '*'
	});
	res.end(JSON.stringify({ error: message, status: statusCode }));
}

function serveStatic(res, filePath, contentType) {
	if (res.headersSent) return;
	if (existsSync(filePath)) {
		res.writeHead(200, { 'Content-Type': contentType });
		const fileData = readFileSync(filePath);
		res.end(fileData);
	} else {
		sendError(res, 404, `${contentType} not found`);
	}
}

function serveHtml(res, filePath, bodyAttributes = {}) {
	if (res.headersSent) return;
	if (existsSync(filePath)) {
		res.writeHead(200, { 'Content-Type': 'text/html' });
		let html = readFileSync(filePath, 'utf8');
		// Inject attributes into body tag
		const attributesString = Object.entries(bodyAttributes)
			.map(([key, value]) => `${key}="${String(value).replace(/"/g, '"')}"`)
			.join(' ');
		if (attributesString) {
			html = html.replace('<body', `<body ${attributesString}`);
		}
		res.end(html);
	} else {
		sendError(res, 404, 'HTML file not found');
	}
}


function getSessionIdFromUrl(req) {
	try {
		const url = new URL(req.url, `http://${req.headers.host}`);
		return url.searchParams.get('sessionId');
	} catch (error) {
		console.error(`Error parsing URL for sessionId: ${error.message}`);
		// Fallback: manual parsing (less reliable)
		const match = req.url.match(/[?&]sessionId=([^&]+)/);
		return match ? match[1] : null;
	}
}

async function handlePostRequest(req, res, callback) {
	let body = '';
	req.on('data', chunk => body += chunk);
	req.on('end', async () => {
		try {
			const parsedBody = JSON.parse(body);
			await callback(parsedBody);
		} catch (error) {
			if (error instanceof SyntaxError) {
				sendError(res, 400, 'Invalid JSON in request body');
			} else {
				console.error('Error handling POST request:', error);
				sendError(res, 500, `Internal Server Error: ${error.message}`);
			}
		}
	});
	req.on('error', (err) => {
		console.error('Request error:', err);
		sendError(res, 500, 'Request error');
	});
}

async function executeDirectTool(res, toolInstance, toolName, toolParams, sessionId) {
	const DEBUG = process.env.DEBUG_CHAT === '1';
	if (DEBUG) {
		console.log(`\n[DEBUG] ===== Direct API Tool Call: ${toolName} =====`);
		console.log(`[DEBUG] Session ID: ${sessionId}`);
		console.log(`[DEBUG] Params:`, toolParams);
	}
	try {
		// Execute the tool instance directly (it handles events/cancellation)
		const result = await toolInstance.execute(toolParams);
		sendJson(res, 200, { results: result, timestamp: new Date().toISOString() });
	} catch (error) {
		console.error(`Error executing direct tool ${toolName}:`, error);
		let statusCode = 500;
		let errorMessage = `Error executing ${toolName}`;
		if (error.message.includes('cancelled')) {
			statusCode = 499; // Client Closed Request
			errorMessage = 'Operation cancelled';
		} else if (error.code === 'ENOENT') {
			statusCode = 404; errorMessage = 'File or path not found';
		} else if (error.code === 'EACCES') {
			statusCode = 403; errorMessage = 'Permission denied';
		}
		// Add more specific error handling if needed
		sendError(res, statusCode, `${errorMessage}: ${error.message}`);
	}
}