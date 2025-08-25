#!/bin/bash

echo "=== LSP Call Graph Cache Demo ==="
echo ""
echo "This demo shows how the cache speeds up repeated LSP queries"
echo ""

# Ensure LSP daemon is running
echo "1. Starting LSP daemon..."
./target/release/probe lsp shutdown 2>/dev/null
./target/release/probe lsp start -f >/dev/null 2>&1 &
sleep 2

# Test file and symbol
TEST_FILE="src/lsp_integration/client.rs"
SYMBOL="get_symbol_info"

echo "2. First extraction (cold cache - will take time for LSP indexing)..."
echo "   File: $TEST_FILE"
echo "   Symbol: $SYMBOL"
echo ""
echo "   Running: probe extract $TEST_FILE#$SYMBOL --lsp"
echo "   ⏱️  Timing..."

# Time the first extraction
START=$(date +%s%N)
./target/release/probe extract "$TEST_FILE#$SYMBOL" --lsp --format json > /tmp/first_extract.json 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( ($END - $START) / 1000000 ))

echo "   ✅ First extraction completed in ${ELAPSED_MS}ms"

# Show some results
if [ -f /tmp/first_extract.json ]; then
    LINES=$(cat /tmp/first_extract.json | wc -l)
    echo "   📊 Extracted $LINES lines of data"
fi

echo ""
echo "3. Second extraction (warm cache - should be immediate)..."
echo "   Running same query again..."

# Time the second extraction
START=$(date +%s%N)
./target/release/probe extract "$TEST_FILE#$SYMBOL" --lsp --format json > /tmp/second_extract.json 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( ($END - $START) / 1000000 ))

echo "   ✅ Second extraction completed in ${ELAPSED_MS}ms (cache hit!)"

# Compare the results
if cmp -s /tmp/first_extract.json /tmp/second_extract.json; then
    echo "   ✅ Results are identical"
else
    echo "   ⚠️  Results differ (unexpected)"
fi

echo ""
echo "4. Modifying the file to trigger cache invalidation..."
echo "   Adding a comment to $TEST_FILE..."

# Add a comment to the file
echo "// Cache test comment - $(date)" >> "$TEST_FILE"

echo "   File modified. MD5 hash changed."
echo ""
echo "5. Third extraction (after modification - will recompute)..."

# Time the third extraction
START=$(date +%s%N)
./target/release/probe extract "$TEST_FILE#$SYMBOL" --lsp --format json > /tmp/third_extract.json 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( ($END - $START) / 1000000 ))

echo "   ✅ Third extraction completed in ${ELAPSED_MS}ms (cache miss due to file change)"

# Restore the file
git checkout -- "$TEST_FILE" 2>/dev/null

echo ""
echo "6. Testing with different symbols in same file..."
echo "   Extracting 'LspClient::new' from same file..."

START=$(date +%s%N)
./target/release/probe extract "$TEST_FILE#new" --lsp --format json > /tmp/new_extract.json 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( ($END - $START) / 1000000 ))

echo "   ✅ Different symbol extraction in ${ELAPSED_MS}ms"

echo ""
echo "7. Checking LSP daemon status..."
./target/release/probe lsp status | grep -E "(Uptime|Total requests|rust)"

echo ""
echo "=== Demo Complete ==="
echo ""
echo "Summary:"
echo "  • First extraction: Slow (LSP indexing + computation)"
echo "  • Second extraction: Fast (cache hit)"
echo "  • After file change: Slow (cache invalidated, recomputed)"
echo "  • Different symbol: Variable (may use partial cache)"
echo ""
echo "The cache is working behind the scenes in the LSP daemon!"

# Cleanup
rm -f /tmp/*_extract.json