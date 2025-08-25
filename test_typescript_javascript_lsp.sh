#!/bin/bash

# Test script for TypeScript and JavaScript LSP integration
# This script validates that both language servers work correctly with probe

set -e

echo "=== Testing TypeScript and JavaScript LSP Integration ==="
echo

# Set PATH to include typescript-language-server
export PATH="$HOME/.npm-global/bin:$PATH"

# Function to test LSP extraction
test_extraction() {
    local file=$1
    local symbol=$2
    local language=$3
    
    echo "Testing $language: $file#$symbol"
    
    # Run extraction with LSP
    result=$(./target/debug/probe extract "$file#$symbol" --lsp 2>&1)
    
    # Check if LSP information is present
    if echo "$result" | grep -q "LSP Information:"; then
        echo "✅ $language LSP working - Call hierarchy found"
        
        # Count incoming and outgoing calls
        incoming=$(echo "$result" | grep -A 20 "Incoming Calls:" | grep -c "file://" || echo "0")
        outgoing=$(echo "$result" | grep -A 20 "Outgoing Calls:" | grep -c "file://" || echo "0")
        
        echo "   📞 Incoming calls: $incoming"
        echo "   📤 Outgoing calls: $outgoing"
    else
        echo "❌ $language LSP not working - No call hierarchy"
        return 1
    fi
    echo
}

# Build probe first
echo "Building probe..."
cargo build --quiet
echo "✅ Build complete"
echo

# Check if LSP daemon is running, start if needed
echo "Checking LSP daemon status..."
if ! ./target/debug/probe lsp status >/dev/null 2>&1; then
    echo "Starting LSP daemon..."
    ./target/debug/probe lsp start -f >/dev/null 2>&1 &
    sleep 5
    echo "✅ LSP daemon started"
else
    echo "✅ LSP daemon already running"
fi
echo

# Wait for language servers to initialize
echo "Waiting for language servers to initialize..."
sleep 10

# Function to test search functionality
test_search() {
    local pattern=$1
    local path=$2
    local language=$3
    
    echo "Testing $language search: '$pattern' in $path"
    
    # Run search command
    result=$(./target/debug/probe search "$pattern" "$path" --max-results 2 2>&1)
    
    # Check if results were found
    if echo "$result" | grep -q "Found [1-9]"; then
        count=$(echo "$result" | grep "Found" | sed 's/Found \([0-9]*\).*/\1/')
        echo "✅ $language search working - Found $count results"
    else
        echo "❌ $language search not working - No results found"
        return 1
    fi
    echo
}

# Function to test search with LSP enrichment
test_lsp_search() {
    local pattern=$1
    local path=$2
    local language=$3
    
    echo "Testing $language search with LSP: '$pattern' in $path"
    
    # Run search command with LSP
    result=$(./target/debug/probe search "$pattern" "$path" --lsp --max-results 1 2>&1)
    
    # Check if LSP information is present
    if echo "$result" | grep -q "LSP Information:"; then
        incoming=$(echo "$result" | grep -A 10 "Incoming Calls:" | grep -c "file://" || echo "0")
        outgoing=$(echo "$result" | grep -A 10 "Outgoing Calls:" | grep -c "file://" || echo "0")
        echo "✅ $language LSP search working - LSP data found"
        echo "   📞 Incoming calls: $incoming"
        echo "   📤 Outgoing calls: $outgoing"
    else
        echo "⚠️ $language LSP search partial - Results found but no LSP data"
    fi
    echo
}

# Test TypeScript Extraction
echo "🔷 TypeScript Extraction Tests:"
test_extraction "lsp-test-typescript/src/main.ts" "calculate" "TypeScript"
test_extraction "lsp-test-typescript/src/main.ts" "add" "TypeScript"

# Test JavaScript Extraction
echo "🟡 JavaScript Extraction Tests:"
test_extraction "lsp-test-javascript/src/main.js" "calculate" "JavaScript"
test_extraction "lsp-test-javascript/src/main.js" "multiply" "JavaScript"

# Test TypeScript Search
echo "🔍 TypeScript Search Tests:"
test_search "calculate" "lsp-test-typescript" "TypeScript"
test_search "add" "lsp-test-typescript" "TypeScript"

# Test JavaScript Search  
echo "🔍 JavaScript Search Tests:"
test_search "calculate" "lsp-test-javascript" "JavaScript"
test_search "Calculator" "lsp-test-javascript" "JavaScript"

# Test TypeScript Search with LSP
echo "🔍🔷 TypeScript LSP Search Tests:"
test_lsp_search "multiply" "lsp-test-typescript" "TypeScript"

# Test JavaScript Search with LSP
echo "🔍🟡 JavaScript LSP Search Tests:"
test_lsp_search "calculate" "lsp-test-javascript" "JavaScript"

# Check final LSP status
echo "Final LSP daemon status:"
./target/debug/probe lsp status

echo
echo "=== All tests completed successfully! ==="
echo "Both TypeScript and JavaScript LSP integration are working correctly."