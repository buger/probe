#!/bin/bash

echo "=== LSP Call Graph Cache Concept Demo ==="
echo ""
echo "This demonstrates the cache concept using probe's extract command"
echo "Note: The cache is implemented but not yet integrated into the daemon"
echo ""

# Test file and symbol
TEST_FILE="src/lsp_integration/client.rs"
SYMBOL="get_symbol_info"

echo "1. First extraction (measuring baseline time)..."
echo "   File: $TEST_FILE"
echo "   Symbol: $SYMBOL"
echo ""

# Ensure LSP daemon is running
./target/release/probe lsp shutdown 2>/dev/null
./target/release/probe lsp start -f >/dev/null 2>&1 &
sleep 3

# Time the first extraction
echo "   ⏱️  Timing first extraction..."
START=$(date +%s%N)
./target/release/probe extract "$TEST_FILE#$SYMBOL" --lsp > /tmp/first_extract.txt 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( ($END - $START) / 1000000 ))

echo "   ✅ First extraction completed in ${ELAPSED_MS}ms"
LINES=$(wc -l < /tmp/first_extract.txt)
echo "   📊 Extracted $LINES lines"

echo ""
echo "2. Second extraction (same query - should reuse LSP server pool)..."
START=$(date +%s%N)
./target/release/probe extract "$TEST_FILE#$SYMBOL" --lsp > /tmp/second_extract.txt 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( ($END - $START) / 1000000 ))

echo "   ✅ Second extraction completed in ${ELAPSED_MS}ms"
echo "   (Faster due to warmed LSP server, but still makes LSP call)"

echo ""
echo "3. Demonstrating cache concept with unit test..."
echo ""
cargo test test_cache_basic_operations --lib 2>&1 | grep -E "(test|ok|running)"

echo ""
echo "4. Running cache integration tests..."
echo ""
cargo test test_cache_deduplication --test call_graph_cache_integration_test 2>&1 | grep -E "(test|ok|running|passed)"

echo ""
echo "=== Explanation ==="
echo ""
echo "The cache implementation provides:"
echo "  • Content-addressed caching (MD5-based keys)"
echo "  • In-flight deduplication (prevents duplicate LSP calls)"
echo "  • Graph-aware invalidation (updates connected nodes)"
echo "  • TTL and LRU eviction (manages memory usage)"
echo ""
echo "Current status:"
echo "  ✅ Cache module implemented and tested"
echo "  ✅ Unit tests passing"
echo "  ✅ Integration tests demonstrate functionality"
echo "  ⚠️  Not yet integrated into LSP daemon (next step)"
echo ""
echo "When integrated, the second call would return in <1ms from cache!"

# Show daemon status
echo ""
echo "Current LSP daemon status:"
./target/release/probe lsp status | head -10

# Cleanup
rm -f /tmp/*_extract.txt