#!/bin/bash

# Test Go with debug output
set -e

echo "=== Starting daemon with debug ==="
RUST_LOG=debug LSP_LOG=1 ./target/release/probe lsp start -f --log-level debug 2>&1 | head -5 &
DAEMON_PID=$!
sleep 3

echo "=== Initialize workspace ==="
./target/release/probe lsp init -w /tmp/go-test --languages go

echo "=== Wait 5 seconds ==="
sleep 5

echo "=== Extract with debug ==="
RUST_LOG=debug ./target/release/probe extract /tmp/go-test/main.go#calculate --lsp 2>&1 | grep -E "Opening|Closing|call_hierarchy" | head -10

echo "=== Check logs ==="
./target/release/probe lsp logs -n 100 | grep -E "Opening|Closing|didOpen|didClose|prepareCallHierarchy.*response" | head -10

# Cleanup
kill $DAEMON_PID 2>/dev/null || true
./target/release/probe lsp shutdown