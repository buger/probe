#!/usr/bin/env python3
"""
Test script to verify LSP daemon socket exhaustion fixes.
This script spawns many parallel LSP searches to test the connection limits.
"""

import asyncio
import subprocess
import sys
import time
from pathlib import Path
import concurrent.futures

async def run_probe_search(query, search_path, timeout=10):
    """Run a single probe search command."""
    try:
        process = await asyncio.create_subprocess_exec(
            "./target/debug/probe", "search", query, str(search_path),
            "--lsp", "--max-results", "5",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE
        )
        
        stdout, stderr = await asyncio.wait_for(process.communicate(), timeout=timeout)
        
        return {
            "returncode": process.returncode,
            "stdout": stdout.decode('utf-8', errors='ignore'),
            "stderr": stderr.decode('utf-8', errors='ignore')
        }
    except asyncio.TimeoutError:
        return {
            "returncode": -1,
            "stdout": "",
            "stderr": "TIMEOUT"
        }
    except Exception as e:
        return {
            "returncode": -2,
            "stdout": "",
            "stderr": str(e)
        }

async def test_concurrent_connections(num_concurrent=100):
    """Test many concurrent connections to the LSP daemon."""
    print(f"Testing {num_concurrent} concurrent LSP connections...")
    
    # Make sure daemon is running first
    print("Starting LSP daemon...")
    try:
        result = subprocess.run(
            ["./target/debug/probe", "lsp", "start", "-f"],
            timeout=5,
            capture_output=True,
            text=True
        )
        print(f"Daemon start result: {result.returncode}")
    except subprocess.TimeoutExpired:
        print("Daemon is starting in background...")
    
    # Wait for daemon to be ready
    await asyncio.sleep(2)
    
    # Create many concurrent search tasks
    tasks = []
    search_queries = ["fn", "struct", "impl", "use", "mod", "let", "match", "if"]
    search_path = Path("./src")
    
    for i in range(num_concurrent):
        query = search_queries[i % len(search_queries)]
        task = run_probe_search(query, search_path, timeout=15)
        tasks.append(task)
    
    # Run all tasks concurrently
    print(f"Running {len(tasks)} concurrent searches...")
    start_time = time.time()
    
    results = await asyncio.gather(*tasks, return_exceptions=True)
    
    end_time = time.time()
    elapsed = end_time - start_time
    
    # Analyze results
    successes = 0
    timeouts = 0
    errors = 0
    connection_errors = 0
    
    for i, result in enumerate(results):
        if isinstance(result, Exception):
            errors += 1
            print(f"Task {i}: Exception - {result}")
        elif result["returncode"] == 0:
            successes += 1
        elif "TIMEOUT" in result["stderr"]:
            timeouts += 1
        elif "connection" in result["stderr"].lower() or "socket" in result["stderr"].lower():
            connection_errors += 1
        else:
            errors += 1
            if result["stderr"]:
                print(f"Task {i}: Error - {result['stderr'][:100]}...")
    
    print(f"\n=== Results after {elapsed:.2f}s ===")
    print(f"Total requests: {num_concurrent}")
    print(f"Successes: {successes}")
    print(f"Timeouts: {timeouts}")
    print(f"Connection errors: {connection_errors}")
    print(f"Other errors: {errors}")
    
    # Check daemon status
    try:
        status_result = subprocess.run(
            ["./target/debug/probe", "lsp", "status"],
            capture_output=True,
            text=True,
            timeout=5
        )
        print(f"\nDaemon status: {status_result.returncode}")
        if status_result.stdout:
            print("Status output:", status_result.stdout[:500])
    except Exception as e:
        print(f"Failed to get daemon status: {e}")
    
    # Success criteria: Most requests should succeed, few connection errors
    success_rate = successes / num_concurrent
    connection_error_rate = connection_errors / num_concurrent
    
    print(f"\nSuccess rate: {success_rate:.2%}")
    print(f"Connection error rate: {connection_error_rate:.2%}")
    
    if success_rate > 0.8 and connection_error_rate < 0.1:
        print("✅ TEST PASSED: Socket limits working correctly")
        return True
    else:
        print("❌ TEST FAILED: Too many connection errors")
        return False

async def main():
    print("LSP Daemon Socket Limit Test")
    print("=" * 40)
    
    # Test with moderate load first
    print("\n🧪 Test 1: 50 concurrent connections")
    success1 = await test_concurrent_connections(50)
    
    await asyncio.sleep(2)  # Let daemon settle
    
    # Test with higher load to check limits
    print("\n🧪 Test 2: 100 concurrent connections (testing limits)")
    success2 = await test_concurrent_connections(100)
    
    # Shutdown daemon
    print("\nShutting down daemon...")
    try:
        subprocess.run(
            ["./target/debug/probe", "lsp", "shutdown"],
            capture_output=True,
            timeout=5
        )
    except:
        pass
    
    if success1 and success2:
        print("\n🎉 ALL TESTS PASSED")
        return 0
    else:
        print("\n💥 SOME TESTS FAILED")
        return 1

if __name__ == "__main__":
    exit_code = asyncio.run(main())
    sys.exit(exit_code)