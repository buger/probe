#!/bin/bash

# Test Go LSP with document open
set -e

echo "=== Cleaning up ==="
./target/release/probe lsp shutdown 2>/dev/null || true
sleep 1

# Create test Go file
mkdir -p /tmp/go-test
cat > /tmp/go-test/main.go << 'EOF'
package main

import "fmt"

func calculate(a, b int) int {
    result := add(a, b)
    result = multiply(result, 2)
    return result
}

func add(x, y int) int {
    return x + y
}

func multiply(x, y int) int {
    return x * y
}

func main() {
    result := calculate(5, 3)
    fmt.Printf("Result: %d\n", result)
}
EOF

cat > /tmp/go-test/go.mod << 'EOF'
module testgo
go 1.21
EOF

echo "=== Starting daemon ==="
./target/release/probe lsp restart 2>/dev/null || ./target/release/probe lsp start
sleep 2

echo "=== Initialize Go workspace ==="
./target/release/probe lsp init -w /tmp/go-test --languages go

echo "=== Wait for gopls initialization (10 seconds) ==="
sleep 10

echo "=== Test extraction with LSP ==="
./target/release/probe extract /tmp/go-test/main.go#calculate --lsp

echo "=== Check logs for document operations ==="
./target/release/probe lsp logs -n 50 | grep -E "Opening document|Closing document|prepareCallHierarchy|incomingCalls|outgoingCalls" | head -10

echo "=== Cleanup ==="
./target/release/probe lsp shutdown