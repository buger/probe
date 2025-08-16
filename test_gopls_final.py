#!/usr/bin/env python3
"""
Final test to diagnose gopls package metadata issue
"""

import subprocess
import json
import time
import os

def send_lsp_request(proc, request):
    """Send an LSP request with proper Content-Length header"""
    request_str = json.dumps(request, separators=(',', ':'))
    content_length = len(request_str.encode('utf-8'))
    message = f"Content-Length: {content_length}\r\n\r\n{request_str}"
    
    proc.stdin.write(message.encode('utf-8'))
    proc.stdin.flush()
    print(f">>> Sent: {request.get('method', 'response')}")
    return request_str

def read_lsp_messages(proc, timeout=2):
    """Read all available LSP messages"""
    import select
    messages = []
    start_time = time.time()
    
    while time.time() - start_time < timeout:
        ready, _, _ = select.select([proc.stdout], [], [], 0.1)
        if not ready:
            continue
        
        # Try to read a message
        try:
            # Read Content-Length header
            header = b""
            while b"\r\n\r\n" not in header:
                byte = proc.stdout.read(1)
                if not byte:
                    break
                header += byte
            
            if not header:
                break
            
            # Parse content length
            header_str = header.decode('utf-8')
            content_length = 0
            for line in header_str.split('\r\n'):
                if line.startswith('Content-Length:'):
                    content_length = int(line.split(':')[1].strip())
                    break
            
            if content_length == 0:
                continue
            
            # Read the content
            content = proc.stdout.read(content_length)
            msg = json.loads(content.decode('utf-8'))
            messages.append(msg)
            
            # Print key messages
            if 'method' in msg:
                if msg['method'] == 'textDocument/publishDiagnostics':
                    diags = msg['params'].get('diagnostics', [])
                    if diags:
                        print(f"<<< Diagnostics: {[d['message'] for d in diags]}")
                else:
                    print(f"<<< {msg['method']}")
            elif 'error' in msg:
                print(f"<<< ERROR: {msg['error']['message']}")
            elif 'result' in msg:
                if msg.get('id') == 1:
                    print(f"<<< Initialized successfully")
                else:
                    print(f"<<< Response for request {msg.get('id')}")
        except:
            break
    
    return messages

def main():
    # Change to the Go project directory
    os.chdir('/Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go')
    print(f"Working directory: {os.getcwd()}")
    print(f"Files: {os.listdir('.')}")
    
    # Read the actual file content
    with open('main.go', 'r') as f:
        file_content = f.read()
    print(f"File content length: {len(file_content)} bytes")
    
    # Start gopls
    print("\n=== Starting gopls ===")
    proc = subprocess.Popen(
        ['gopls', 'serve', '-mode=stdio'],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=open('/tmp/gopls_final.err', 'w'),
        cwd='/Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go'
    )
    
    time.sleep(0.5)
    
    # Initialize
    init_request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": None,
            "rootUri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go",
            "capabilities": {
                "textDocument": {
                    "callHierarchy": {"dynamicRegistration": False}
                }
            },
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
    }
    
    print("\n1. Sending initialize...")
    send_lsp_request(proc, init_request)
    messages = read_lsp_messages(proc, timeout=3)
    
    # Send initialized
    print("\n2. Sending initialized notification...")
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })
    
    time.sleep(2)  # Wait for gopls to load packages
    messages = read_lsp_messages(proc, timeout=1)
    
    # Open document with actual file content
    print("\n3. Opening document with actual content...")
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go",
                "languageId": "go",
                "version": 1,
                "text": file_content
            }
        }
    })
    
    time.sleep(3)  # Wait for gopls to process
    messages = read_lsp_messages(proc, timeout=2)
    
    # Try call hierarchy
    print("\n4. Requesting call hierarchy for Calculate function...")
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/prepareCallHierarchy",
        "params": {
            "textDocument": {
                "uri": "file:///Users/leonidbugaev/conductor/repo/probe/paris/lsp-test-go/main.go"
            },
            "position": {
                "line": 9,  # Calculate function line
                "character": 6  # 'C' in Calculate
            }
        }
    })
    
    messages = read_lsp_messages(proc, timeout=5)
    
    # Look for the response
    for msg in messages:
        if msg.get('id') == 2:
            if 'error' in msg:
                print(f"\n!!! Call hierarchy ERROR: {msg['error']['message']}")
            elif 'result' in msg:
                print(f"\n!!! Call hierarchy SUCCESS: {json.dumps(msg['result'], indent=2)[:200]}...")
    
    # Shutdown
    send_lsp_request(proc, {
        "jsonrpc": "2.0",
        "id": 999,
        "method": "shutdown",
        "params": None
    })
    
    time.sleep(1)
    proc.terminate()
    
    print("\n=== Checking stderr for clues ===")
    with open('/tmp/gopls_final.err', 'r') as f:
        lines = f.readlines()
        for line in lines[-20:]:
            if 'error' in line.lower() or 'package' in line.lower() or 'metadata' in line.lower():
                print(line.rstrip())

if __name__ == "__main__":
    main()