#!/bin/bash
# Check for missing documentation and save results

set -e

cd "$(dirname "$0")/.."

echo "Checking for missing documentation..."

# Run cargo doc with missing_docs warnings
env RUSTDOCFLAGS="-W missing_docs" cargo doc --no-deps 2>&1 | tee /tmp/doc_check_full.log

# Count warnings
WARNINGS=$(grep -c "warning: missing documentation" /tmp/doc_check_full.log || echo "0")

echo ""
echo "================================================"
echo "Missing documentation warnings: $WARNINGS"
echo "================================================"

# Save first 150 warnings with context to a separate file
grep "warning: missing documentation" -A 2 /tmp/doc_check_full.log | head -150 > /tmp/doc_check_sample.txt

echo "Full log saved to: /tmp/doc_check_full.log"
echo "Sample (first 150 warnings) saved to: /tmp/doc_check_sample.txt"

exit 0
