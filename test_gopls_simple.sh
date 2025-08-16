#!/bin/bash

# Simple test for gopls integration
set -e

echo "=== Cleaning up ==="
./target/release/probe lsp shutdown 2>/dev/null || true
sleep 1

# Create test Go file
mkdir -p /tmp/go-test
cat > /tmp/go-test/main.go << 'EOF'
package main

func calculate(a, b int) int {
    return a + b
}

func main() {
    result := calculate(5, 3)
    println(result)
}
EOF

cat > /tmp/go-test/go.mod << 'EOF'
module test
go 1.21
EOF

echo "=== Starting daemon ==="
LSP_LOG=1 ./target/release/probe lsp start -f --log-level debug &
DAEMON_PID=$!
sleep 2

echo "=== Daemon status ==="
./target/release/probe lsp status

echo "=== Initializing Go workspace ==="
./target/release/probe lsp init -w /tmp/go-test --languages go

echo "=== Waiting for gopls to initialize (15 seconds) ==="
sleep 15

echo "=== Testing extraction with LSP ==="
LSP_LOG=1 ./target/release/probe extract /tmp/go-test/main.go#calculate --lsp

echo "=== Checking logs for progress events ==="
./target/release/probe lsp logs -n 100 | grep -E "Progress|workDone|CallHierarchy|index|Loading" | head -20

# Cleanup
kill $DAEMON_PID 2>/dev/null || true
./target/release/probe lsp shutdown 2>/dev/null || true