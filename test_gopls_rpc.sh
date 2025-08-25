#!/bin/bash

# Script to test gopls with RPC tracing and compare with our daemon

echo "=== Starting gopls in serve mode with RPC tracing ==="

# Create a test script that sends proper LSP requests
cat > /tmp/gopls_test_requests.txt << 'EOF'
Content-Length: 180

{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":"file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go","capabilities":{},"initializationOptions":{"expandWorkspaceToModule":true}}}
Content-Length: 52

{"jsonrpc":"2.0","method":"initialized","params":{}}
Content-Length: 134

{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go","languageId":"go","version":1,"text":"package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(Calculate(5, 3))\n}\n\nfunc Calculate(a, b int) int {\n\treturn Add(a, b) + Multiply(a, b)\n}\n\nfunc Add(a, b int) int {\n\treturn a + b\n}\n\nfunc Multiply(a, b int) int {\n\treturn a * b\n}\n"}}}
Content-Length: 151

{"jsonrpc":"2.0","id":2,"method":"textDocument/prepareCallHierarchy","params":{"textDocument":{"uri":"file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go"},"position":{"line":9,"character":6}}}
Content-Length: 218

{"jsonrpc":"2.0","id":3,"method":"callHierarchy/incomingCalls","params":{"item":{"name":"Calculate","kind":12,"uri":"file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go","range":{"start":{"line":9,"character":0},"end":{"line":11,"character":1}},"selectionRange":{"start":{"line":9,"character":5},"end":{"line":9,"character":14}}}}}
Content-Length: 58

{"jsonrpc":"2.0","id":999,"method":"shutdown","params":null}
Content-Length: 46

{"jsonrpc":"2.0","method":"exit","params":null}
EOF

echo "Test requests created at /tmp/gopls_test_requests.txt"

# Run gopls with RPC tracing
echo ""
echo "=== Running gopls with RPC trace (working directory: lsp-test-go) ==="
cd /Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go
cat /tmp/gopls_test_requests.txt | gopls serve -mode=stdio -rpc.trace 2>/tmp/gopls_rpc_trace.log 1>/tmp/gopls_responses.log

echo ""
echo "=== RPC Trace (first 100 lines) ==="
head -100 /tmp/gopls_rpc_trace.log

echo ""
echo "=== Responses (first 50 lines) ==="
head -50 /tmp/gopls_responses.log | jq -r 'select(.result != null) | .result' 2>/dev/null || head -50 /tmp/gopls_responses.log

echo ""
echo "=== Looking for call hierarchy results in responses ==="
grep -A10 "prepareCallHierarchy\|incomingCalls" /tmp/gopls_responses.log | head -30