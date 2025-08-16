#!/bin/bash

# Test Rust LSP still works without regression
set -e

echo "=== Cleaning up ==="
./target/release/probe lsp shutdown 2>/dev/null || true
sleep 1

# Create test Rust file
mkdir -p /tmp/rust-test/src
cat > /tmp/rust-test/src/main.rs << 'EOF'
fn main() {
    let result = calculate(5, 3);
    println!("Result: {}", result);
}

fn calculate(a: i32, b: i32) -> i32 {
    let sum = add(a, b);
    multiply(sum, 2)
}

fn add(x: i32, y: i32) -> i32 {
    x + y
}

fn multiply(x: i32, y: i32) -> i32 {
    x * y
}
EOF

cat > /tmp/rust-test/Cargo.toml << 'EOF'
[package]
name = "rust-test"
version = "0.1.0"
edition = "2021"
EOF

echo "=== Starting daemon ==="
./target/release/probe lsp restart 2>/dev/null || ./target/release/probe lsp start
sleep 2

echo "=== Initialize Rust workspace ==="
./target/release/probe lsp init -w /tmp/rust-test --languages rust

echo "=== Wait for rust-analyzer (5 seconds) ==="
sleep 5

echo "=== Test extraction with LSP (should include call hierarchy) ==="
time ./target/release/probe extract /tmp/rust-test/src/main.rs#calculate --lsp

echo -e "\n=== Check that no gopls-specific logic was triggered ==="
./target/release/probe lsp logs -n 50 | grep -E "Opening document for gopls|Retrying call hierarchy|Waiting for gopls" | head -5 || echo "Good: No gopls-specific messages for Rust"

echo -e "\n=== Verify call hierarchy is present ==="
./target/release/probe lsp logs -n 50 | grep -E "incomingCalls|outgoingCalls" | head -5

echo "=== Cleanup ==="
./target/release/probe lsp shutdown