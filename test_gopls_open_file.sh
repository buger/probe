#!/bin/bash

# Test gopls with proper file opening
set -e

echo "=== Cleaning up ==="
./target/release/probe lsp shutdown 2>/dev/null || true
sleep 1

# Create test Go file
mkdir -p /tmp/go-open-test
cat > /tmp/go-open-test/main.go << 'EOF'
package main

import "fmt"

func calculate(a, b int) int {
    return add(a, b) * 2
}

func add(x, y int) int {
    return x + y
}

func main() {
    result := calculate(5, 3)
    fmt.Println("Result:", result)
}
EOF

cat > /tmp/go-open-test/go.mod << 'EOF'
module opentest
go 1.21
EOF

echo "=== Starting daemon ==="
./target/release/probe lsp start 2>/dev/null
sleep 2

echo "=== Initialize Go workspace ==="
./target/release/probe lsp init -w /tmp/go-open-test --languages go

echo "=== Wait for gopls (20 seconds) ==="
sleep 20

echo "=== Test extraction ==="
./target/release/probe extract /tmp/go-open-test/main.go#calculate --lsp

echo "=== Check if we got any responses ==="
./target/release/probe lsp logs -n 100 | grep -E "prepareCallHierarchy.*response|FROM LSP.*result.*\\[\\]|FROM LSP.*result.*null" | head -5

# Cleanup
./target/release/probe lsp shutdown