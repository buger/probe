#!/bin/bash

# Test transparent gopls fix
set -e

echo "=== Cleaning up ==="
./target/release/probe lsp shutdown 2>/dev/null || true
sleep 1

# Create test Go file if needed
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

echo "=== Wait 5 seconds for gopls ==="
sleep 5

echo "=== Test extraction with LSP (should work transparently) ==="
time ./target/release/probe extract /tmp/go-test/main.go#calculate --lsp

echo -e "\n=== Check logs for retry behavior ==="
./target/release/probe lsp logs -n 100 | grep -E "Retrying|no package metadata|Opening document for gopls|Waiting for gopls" | head -10 || echo "No retry messages found"

echo -e "\n=== Test again (should be faster, document already open) ==="
time ./target/release/probe extract /tmp/go-test/main.go#add --lsp

echo "=== Cleanup ==="
./target/release/probe lsp shutdown