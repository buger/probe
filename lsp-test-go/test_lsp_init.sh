#!/bin/bash

# Send proper LSP messages with headers
send_lsp_message() {
    local content="$1"
    local length=${#content}
    printf "Content-Length: %d\r\n\r\n%s" "$length" "$content"
}

# Initialize
init_msg='{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "processId": null,
    "rootUri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go",
    "rootPath": "/Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go",
    "workspaceFolders": [{
      "uri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go",
      "name": "lsp-test-go"
    }],
    "initializationOptions": {
      "expandWorkspaceToModule": true,
      "directoryFilters": ["-", "+."],
      "experimentalWorkspaceModule": false
    },
    "capabilities": {
      "workspace": {
        "configuration": true,
        "workspaceFolders": true
      },
      "textDocument": {
        "callHierarchy": {
          "dynamicRegistration": false
        }
      }
    }
  }
}'

# Initialized notification
initialized_msg='{
  "jsonrpc": "2.0",
  "method": "initialized",
  "params": {}
}'

# Test sequence
(
  send_lsp_message "$init_msg"
  sleep 1
  send_lsp_message "$initialized_msg"
  sleep 2
) | gopls serve -mode=stdio -vv 2>&1 | grep -A5 -B5 "go list"