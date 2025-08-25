#!/bin/bash

# Debug gopls to see if we can get call hierarchy
set -e

echo "=== Starting daemon with debug logging ==="
RUST_LOG=debug LSP_LOG=1 ./target/release/probe lsp start -f --log-level debug 2>&1 | head -10 &
DAEMON_PID=$!
sleep 3

echo "=== Initialize Go workspace ==="
./target/release/probe lsp init -w /tmp/go-test --languages go

echo "=== Wait longer for gopls to fully initialize (15 seconds) ==="
for i in {1..3}; do
    echo "  Waiting... ($((i*5))/15 seconds)"
    sleep 5
done

echo "=== Check gopls status ==="
./target/release/probe lsp status

echo "=== Try extraction with debug output ==="
RUST_LOG=debug ./target/release/probe extract /tmp/go-test/main.go#calculate --lsp 2>&1 | grep -E "call_hierarchy|no package metadata|Retrying|Opening document" | head -20

echo "=== Check detailed logs ==="
./target/release/probe lsp logs -n 200 | grep -E "didOpen|FROM LSP.*result.*calculate|FROM LSP.*error|prepareCallHierarchy" | head -30

echo "=== Try a simpler approach - just test if gopls responds to didOpen ==="
./target/release/probe lsp logs -n 200 | grep -E "didOpen.*main.go|publishDiagnostics.*main.go" | head -10

# Cleanup
kill $DAEMON_PID 2>/dev/null || true
./target/release/probe lsp shutdown 2>/dev/null || true