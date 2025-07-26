/**
 * @buger/probe-chat
 * CLI chat interface for Probe code search
 */

import { randomUUID } from 'crypto';
import { generateText } from 'ai';
import { anthropic } from '@ai-sdk/anthropic';
import { openai } from '@ai-sdk/openai';
import { google } from '@ai-sdk/google';
import { existsSync } from 'fs';
import { tools } from '@buger/probe';

/**
 * ProbeChat class to handle chat interactions with AI models
 */
export class ProbeChat {
  /**
   * Create a new ProbeChat instance
   * @param {Object} options - Configuration options
   * @param {boolean} options.debug - Enable debug mode
   * @param {string} options.model - Model name to use
   * @param {string} options.anthropicApiKey - Anthropic API key
   * @param {string} options.openaiApiKey - OpenAI API key
   * @param {string} options.googleApiKey - Google API key
   * @param {string} options.anthropicApiUrl - Custom Anthropic API URL
   * @param {string} options.openaiApiUrl - Custom OpenAI API URL
   * @param {string} options.googleApiUrl - Custom Google API URL
   * @param {Array<string>} options.allowedFolders - Folders to search in
   */
  constructor(options = {}) {
    // Initialize options with defaults
    this.options = {
      debug: process.env.DEBUG_CHAT === 'true' || process.env.DEBUG_CHAT === '1' || options.debug,
      model: options.model || process.env.MODEL_NAME,
      anthropicApiKey: options.anthropicApiKey || process.env.ANTHROPIC_API_KEY,
      openaiApiKey: options.openaiApiKey || process.env.OPENAI_API_KEY,
      googleApiKey: options.googleApiKey || process.env.GOOGLE_API_KEY || process.env.GEMINI_API_KEY,
      anthropicApiUrl: options.anthropicApiUrl || process.env.ANTHROPIC_API_URL || 'https://api.anthropic.com/v1',
      openaiApiUrl: options.openaiApiUrl || process.env.OPENAI_API_URL || 'https://api.openai.com/v1',
      googleApiUrl: options.googleApiUrl || process.env.GOOGLE_API_URL,
      allowedFolders: options.allowedFolders || (process.env.ALLOWED_FOLDERS ?
        process.env.ALLOWED_FOLDERS.split(',').map(folder => folder.trim()).filter(Boolean) : [])
    };

    // Initialize token counter
    this.requestTokens = 0;
    this.responseTokens = 0;

    // Generate a unique session ID for this chat instance
    this.sessionId = randomUUID();

    if (this.options.debug) {
      console.log(`[DEBUG] Generated session ID for chat: ${this.sessionId}`);
    }

    // Store the session ID in an environment variable for tools to access
    process.env.PROBE_SESSION_ID = this.sessionId;

    // Initialize the chat model
    this.initializeModel();

    // Initialize chat history
    this.history = [];

    // Maximum number of messages to keep in history
    this.MAX_HISTORY_MESSAGES = 20;
  }

  /**
   * Initialize the AI model based on available API keys
   */
  initializeModel() {
    // Determine which API to use based on available keys
    if (this.options.anthropicApiKey) {
      // Configure the anthropic provider with API key
      this.provider = (modelName) => {
        // Set the API key in the environment variable
        process.env.ANTHROPIC_API_KEY = this.options.anthropicApiKey;
        if (this.options.anthropicApiUrl) {
          process.env.ANTHROPIC_API_URL = this.options.anthropicApiUrl;
        }
        return anthropic(modelName);
      };

      this.model = this.options.model || 'claude-3-7-sonnet-latest';
      this.apiType = 'anthropic';

      if (this.options.debug) {
        console.log(`[DEBUG] Using Anthropic API with model: ${this.model}`);
      }
    } else if (this.options.openaiApiKey) {
      // Configure the openai provider with API key
      this.provider = (modelName) => {
        // Set the API key in the environment variable
        process.env.OPENAI_API_KEY = this.options.openaiApiKey;
        if (this.options.openaiApiUrl) {
          process.env.OPENAI_API_URL = this.options.openaiApiUrl;
        }
        return openai(modelName);
      };

      this.model = this.options.model || 'gpt-4o-2024-05-13';
      this.apiType = 'openai';

      if (this.options.debug) {
        console.log(`[DEBUG] Using OpenAI API with model: ${this.model}`);
      }
    } else if (this.options.googleApiKey) {
      // Configure the google provider with API key
      this.provider = (modelName) => {
        // Set the API key in the environment variable (both names for compatibility)
        process.env.GOOGLE_API_KEY = this.options.googleApiKey;
        process.env.GEMINI_API_KEY = this.options.googleApiKey;
        if (this.options.googleApiUrl) {
          process.env.GOOGLE_API_URL = this.options.googleApiUrl;
        }
        return google(modelName);
      };

      this.model = this.options.model || 'gemini-2.5-pro-preview-06-05';
      this.apiType = 'google';

      if (this.options.debug) {
        console.log(`[DEBUG] Using Google API with model: ${this.model}`);
      }
    } else {
      throw new Error('No API key provided. Please set ANTHROPIC_API_KEY, OPENAI_API_KEY, or GOOGLE_API_KEY environment variable.');
    }
  }

  /**
   * Get the session ID
   * @returns {string} - The session ID
   */
  getSessionId() {
    return this.sessionId;
  }

  /**
   * Get token usage statistics
   * @returns {Object} - Token usage statistics
   */
  getTokenUsage() {
    return {
      request: this.requestTokens,
      response: this.responseTokens,
      total: this.requestTokens + this.responseTokens
    };
  }

  /**
   * Get the system message with instructions for the AI
   * @returns {string} - The system message
   */
  getSystemMessage() {
    let systemMessage = tools.DEFAULT_SYSTEM_MESSAGE;

    if (this.options.allowedFolders.length > 0) {
      const folderList = this.options.allowedFolders.map(f => `"${f}"`).join(', ');
      systemMessage += `\n\nThe following folders are configured for code search: ${folderList}. When using search, specify one of these folders in the path argument.`;
    } else {
      systemMessage += `\n\nNo specific folders are configured for code search, so the current directory will be used by default. You can omit the path parameter in your search calls, or use '.' to explicitly search in the current directory.`;
    }

    return systemMessage;
  }

  /**
   * Count tokens in a text (approximate)
   * @param {string} text - The text to count tokens for
   * @returns {number} - The approximate token count
   */
  countTokens(text) {
    // Rough approximation: 1 token ≈ 4 characters for English text
    return Math.ceil(text.length / 4);
  }

  /**
   * Process a user message and get a response
   * @param {string} message - The user message
   * @returns {Promise<string>} - The AI response
   */
  async chat(message) {
    try {
      if (this.options.debug) {
        console.log(`[DEBUG] Received user message: ${message}`);
      }

      // Count tokens in the user message
      const messageTokens = this.countTokens(message);
      this.requestTokens += messageTokens;

      // Limit history to prevent token overflow
      if (this.history.length > this.MAX_HISTORY_MESSAGES) {
        const historyStart = this.history.length - this.MAX_HISTORY_MESSAGES;
        this.history = this.history.slice(historyStart);

        if (this.options.debug) {
          console.log(`[DEBUG] Trimmed history to ${this.history.length} messages`);
        }
      }

      // Prepare messages array
      const messages = [
        ...this.history,
        { role: 'user', content: message }
      ];

      if (this.options.debug) {
        console.log(`[DEBUG] Sending ${messages.length} messages to model`);
      }

      // Configure generateText options
      const generateOptions = {
        model: this.provider(this.model),
        messages: messages,
        system: this.getSystemMessage(),
        tools: {
          search: tools.searchTool,
          query: tools.queryTool,
          extract: tools.extractTool
        },
        maxSteps: 15,
        temperature: 0.7
      };

      // Add API-specific options
      if (this.apiType === 'anthropic' && this.model.includes('3-7')) {
        generateOptions.experimental_thinking = {
          enabled: true,
          budget: 8000
        };
      }

      // Generate response
      const result = await generateText(generateOptions);

      // Add the response to history
      this.history.push({ role: 'user', content: message });
      this.history.push({ role: 'assistant', content: result.text });

      // Count tokens in the response
      const responseTokens = this.countTokens(result.text);
      this.responseTokens += responseTokens;

      // Log tool usage
      if (result.toolCalls && result.toolCalls.length > 0 && this.options.debug) {
        console.log(`[DEBUG] Tool was used: ${result.toolCalls.length} times`);
        result.toolCalls.forEach((call, index) => {
          console.log(`[DEBUG] Tool call ${index + 1}: ${call.name}`);
        });
      }

      return result.text;
    } catch (error) {
      console.error('Error generating response:', error);
      throw error;
    }
  }

  /**
   * Clear the chat history
   */
  clearHistory() {
    this.history = [];
    this.sessionId = randomUUID();
    process.env.PROBE_SESSION_ID = this.sessionId;

    if (this.options.debug) {
      console.log(`[DEBUG] Chat history cleared, new session ID: ${this.sessionId}`);
    }

    return this.sessionId;
  }
}

// Export the ProbeChat class
export default ProbeChat;

// Export the tools from @buger/probe for convenience
export { tools }; 