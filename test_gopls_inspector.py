#!/usr/bin/env python3
"""
Test gopls with proper LSP communication and capture RPC trace in LSP Inspector format
"""

import subprocess
import json
import sys
import time
import os

def send_lsp_request(proc, request):
    """Send an LSP request with proper Content-Length header"""
    request_str = json.dumps(request)
    content_length = len(request_str.encode('utf-8'))
    message = f"Content-Length: {content_length}\r\n\r\n{request_str}"
    
    proc.stdin.write(message.encode('utf-8'))
    proc.stdin.flush()
    print(f"Sent: {request['method'] if 'method' in request else 'response'}")

def read_lsp_response(proc, timeout=5):
    """Read an LSP response with Content-Length header"""
    import select
    
    # Use select to check if data is available
    ready, _, _ = select.select([proc.stdout], [], [], timeout)
    if not ready:
        return None
    
    # Read Content-Length header
    header = b""
    while b"\r\n\r\n" not in header:
        byte = proc.stdout.read(1)
        if not byte:
            return None
        header += byte
    
    # Parse content length
    header_str = header.decode('utf-8')
    content_length = 0
    for line in header_str.split('\r\n'):
        if line.startswith('Content-Length:'):
            content_length = int(line.split(':')[1].strip())
            break
    
    if content_length == 0:
        return None
    
    # Read the content
    content = proc.stdout.read(content_length)
    return json.loads(content.decode('utf-8'))

def main():
    # Change to the Go project directory
    os.chdir('/Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go')
    
    # Start gopls with RPC tracing
    print("Starting gopls with RPC tracing...")
    proc = subprocess.Popen(
        ['gopls', 'serve', '-mode=stdio', '-rpc.trace'],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=open('/tmp/gopls_inspector.log', 'w'),
        cwd='/Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go'
    )
    
    time.sleep(1)
    
    # Initialize
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": None,
            "rootUri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go",
            "capabilities": {},
            "initializationOptions": {
                "expandWorkspaceToModule": True,
                "directoryFilters": ["-", "+."],
                "symbolScope": "workspace"
            },
            "workspaceFolders": [{
                "uri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go",
                "name": "lsp-test-go"
            }]
        }
    })
    
    response = read_lsp_response(proc, timeout=10)
    if response:
        print(f"Initialize response: {json.dumps(response, indent=2)[:200]}...")
    
    # Send initialized notification
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })
    
    time.sleep(2)  # Give gopls time to process
    
    # Open the document
    with open('/Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go', 'r') as f:
        content = f.read()
    
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go",
                "languageId": "go",
                "version": 1,
                "text": content
            }
        }
    })
    
    time.sleep(5)  # Wait for gopls to load packages
    
    # Try prepareCallHierarchy
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": {
                "uri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go"
            },
            "position": {
                "line": 9,
                "character": 6
            }
        }
    })
    
    response = read_lsp_response(proc, timeout=10)
    if response:
        print(f"PrepareCallHierarchy response: {json.dumps(response, indent=2)}")
        
        if 'result' in response and response['result']:
            # Get incoming calls
            item = response['result'][0]
            send_lsp_request(proc, {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "callHierarchy/incomingCalls",
                "params": {
                    "item": item
                }
            })
            
            response = read_lsp_response(proc, timeout=10)
            if response:
                print(f"IncomingCalls response: {json.dumps(response, indent=2)}")
    
    # Shutdown
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "id": 999,
        "method": "shutdown",
        "params": None
    })
    
    read_lsp_response(proc, timeout=5)
    
    # Exit
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "method": "exit",
        "params": None
    })
    
    proc.terminate()
    proc.wait()
    
    print("\n=== RPC Trace (LSP Inspector format) ===")
    with open('/tmp/gopls_inspector.log', 'r') as f:
        lines = f.readlines()
        for line in lines[:50]:  # First 50 lines
            print(line.rstrip())

if __name__ == "__main__":
    main()