#!/bin/bash

LOG_DIR=".check-logs"
mkdir -p "$LOG_DIR"

echo "Running checks in parallel..."

cargo test --lib > "$LOG_DIR/test-lib.log" 2>&1 &
PID1=$!

cargo test --test '*' > "$LOG_DIR/test-integration.log" 2>&1 &
PID2=$!

cargo clippy --no-deps --all-targets -- -D warnings > "$LOG_DIR/clippy.log" 2>&1 &
PID3=$!

wait $PID1; EXIT1=$?
wait $PID2; EXIT2=$?
wait $PID3; EXIT3=$?

echo ""
echo "=== Results ==="
if [ $EXIT1 -eq 0 ]; then
    echo "✓ Library tests passed"
else
    echo "✗ Library tests failed (exit $EXIT1). See $LOG_DIR/test-lib.log"
fi

if [ $EXIT2 -eq 0 ]; then
    echo "✓ Integration tests passed"
else
    echo "✗ Integration tests failed (exit $EXIT2). See $LOG_DIR/test-integration.log"
fi

if [ $EXIT3 -eq 0 ]; then
    echo "✓ Clippy passed"
else
    echo "✗ Clippy failed (exit $EXIT3). See $LOG_DIR/clippy.log"
fi

TOTAL_EXIT=$((EXIT1 + EXIT2 + EXIT3))
if [ $TOTAL_EXIT -eq 0 ]; then
    echo ""
    echo "All checks passed!"
fi

exit $TOTAL_EXIT
