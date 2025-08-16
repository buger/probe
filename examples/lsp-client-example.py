#!/usr/bin/env python3
"""
Example LSP daemon client implementation.
Demonstrates how to connect to the probe LSP daemon and use its services.

Requirements:
    None (uses standard library only)

Usage:
    python lsp-client-example.py
"""

import socket
import struct
import json
import uuid
import os
import sys
import time

class LspDaemonClient:
    def __init__(self):
        self.socket = None
        self.socket_path = self._get_socket_path()
        print(f"Socket path: {self.socket_path}")
    
    def _get_socket_path(self):
        """Get platform-specific socket path"""
        if os.name == 'nt':  # Windows
            return r'\\.\pipe\lsp-daemon'
        else:  # Unix/macOS
            temp_dir = os.environ.get('TMPDIR', '/tmp')
            # Remove trailing slash if present
            temp_dir = temp_dir.rstrip('/')
            return f"{temp_dir}/lsp-daemon.sock"
    
    def connect(self):
        """Connect to the daemon"""
        # Try to connect first
        for attempt in range(3):
            try:
                # Unix domain socket
                self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.socket.connect(self.socket_path)
                break  # Connection successful
            except (ConnectionRefusedError, FileNotFoundError) as e:
                if attempt == 0:
                    print(f"Daemon not running, starting it...")
                    # Try to find probe binary
                    probe_cmd = "./target/debug/probe" if os.path.exists("./target/debug/probe") else "probe"
                    result = os.system(f"{probe_cmd} lsp start 2>/dev/null")
                    if result == 0:
                        print("Daemon started, waiting for it to be ready...")
                    time.sleep(3)  # Give daemon more time to start
                elif attempt < 2:
                    print(f"Waiting for daemon to start...")
                    time.sleep(1)
                else:
                    raise
        
        # Send Connect message
        client_id = str(uuid.uuid4())
        request = {
            "type": "Connect",
            "client_id": client_id
        }
        response = self._send_request(request)
        
        if response.get("type") == "Connected":
            daemon_version = response.get("daemon_version", "unknown")
            print(f"✓ Connected to daemon v{daemon_version}")
        else:
            print(f"Unexpected response: {response}")
        
        return client_id
    
    def _send_request(self, request):
        """Send request and receive response"""
        # Encode as JSON
        json_str = json.dumps(request)
        encoded = json_str.encode('utf-8')
        
        # Prepend length (4 bytes, big-endian)
        length = struct.pack('>I', len(encoded))
        
        # Send length + message
        self.socket.sendall(length + encoded)
        
        # Read response length
        length_bytes = self._recv_exact(4)
        response_length = struct.unpack('>I', length_bytes)[0]
        
        # Read response
        response_bytes = self._recv_exact(response_length)
        
        # Decode JSON
        json_str = response_bytes.decode('utf-8')
        return json.loads(json_str)
    
    def _recv_exact(self, n):
        """Receive exactly n bytes"""
        data = b''
        while len(data) < n:
            chunk = self.socket.recv(n - len(data))
            if not chunk:
                raise ConnectionError("Socket closed")
            data += chunk
        return data
    
    def get_status(self):
        """Get daemon status"""
        request = {
            "type": "Status",
            "request_id": str(uuid.uuid4())
        }
        response = self._send_request(request)
        
        if response.get("type") == "Status":
            return response.get("status", {})
        elif response.get("type") == "Error":
            raise Exception(f"Error: {response.get('error', 'Unknown error')}")
        else:
            raise Exception(f"Unexpected response: {response}")
    
    def ping(self):
        """Ping the daemon"""
        request = {
            "type": "Ping",
            "request_id": str(uuid.uuid4())
        }
        response = self._send_request(request)
        return response.get("type") == "Pong"
    
    def get_logs(self, lines=10):
        """Get daemon logs"""
        request = {
            "type": "GetLogs",
            "request_id": str(uuid.uuid4()),
            "lines": lines
        }
        response = self._send_request(request)
        
        if response.get("type") == "Logs":
            return response.get("entries", [])
        else:
            return []
    
    def get_call_hierarchy(self, file_path, line, column):
        """Get call hierarchy for a symbol"""
        request = {
            "type": "CallHierarchy",
            "request_id": str(uuid.uuid4()),
            "file_path": file_path,
            "line": line,
            "column": column,
            "workspace_hint": None
        }
        response = self._send_request(request)
        
        if response.get("type") == "CallHierarchy":
            return response.get("result", {})
        elif response.get("type") == "Error":
            return {"error": response.get("error", "Unknown error")}
        else:
            return {"error": f"Unexpected response: {response}"}
    
    def close(self):
        """Close the connection"""
        if self.socket:
            self.socket.close()
            self.socket = None

def main():
    """Example usage of the LSP daemon client"""
    client = LspDaemonClient()
    
    try:
        # Connect to daemon
        print("Connecting to LSP daemon...")
        client_id = client.connect()
        print(f"Client ID: {client_id}")
        print()
        
        # Ping test
        print("Testing ping...")
        if client.ping():
            print("✓ Ping successful")
        print()
        
        # Get status
        print("Getting daemon status...")
        status = client.get_status()
        print(f"  Uptime: {status['uptime_secs']}s")
        print(f"  Total requests: {status['total_requests']}")
        print(f"  Active connections: {status['active_connections']}")
        print(f"  Version: {status['version']}")
        print(f"  Git hash: {status['git_hash']}")
        
        # Show server pools
        if status['pools']:
            print("\nLanguage servers:")
            for pool in status['pools']:
                print(f"  - {pool['language']}: {pool['status']}")
        print()
        
        # Get recent logs
        print("Recent daemon logs:")
        logs = client.get_logs(5)
        for entry in logs:
            level = entry['level']
            message = entry['message'][:80]  # Truncate long messages
            print(f"  [{level}] {message}")
        print()
        
        # Test call hierarchy (if we have a Rust file)
        test_file = "lsp-test-project/src/main.rs"
        if os.path.exists(test_file):
            print(f"Testing call hierarchy for {test_file}...")
            result = client.get_call_hierarchy(test_file, 66, 4)  # calculate_result function
            
            if "error" in result:
                print(f"  Error: {result['error']}")
            else:
                incoming = result.get('incoming_calls', [])
                outgoing = result.get('outgoing_calls', [])
                print(f"  Incoming calls: {len(incoming)}")
                print(f"  Outgoing calls: {len(outgoing)}")
                
                if incoming:
                    print("  Callers:")
                    for call in incoming[:3]:  # Show first 3
                        name = call.get('name', 'unknown')
                        file = call.get('uri', '').split('/')[-1]
                        print(f"    - {name} in {file}")
        
    except Exception as e:
        print(f"Error: {e}")
        import traceback
        traceback.print_exc()
    
    finally:
        client.close()
        print("\nConnection closed")

if __name__ == "__main__":
    main()