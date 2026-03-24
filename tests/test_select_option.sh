#!/usr/bin/env bash
# Integration test: verify that select_option key sequences
# correctly navigate a selection UI via tmux send-keys.
#
# This simulates what claude-manager does when you press 1/2/3
# on a session waiting for approval.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SESSION="cm-test-$$"
RESULT_FILE="/tmp/cm_test_result_$$"
PASSED=0
FAILED=0

cleanup() {
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    rm -f "$RESULT_FILE"
}
trap cleanup EXIT

send_option() {
    local target="$1"
    local option="$2"
    # Replicate select_option logic: (option-1) Down presses, then Enter
    for ((i = 1; i < option; i++)); do
        tmux send-keys -t "$target" Down
        sleep 0.05
    done
    sleep 0.05
    tmux send-keys -t "$target" Enter
}

run_test() {
    local option="$1"
    local expected="$2"

    rm -f "$RESULT_FILE"

    # Start fake prompt in tmux
    tmux new-session -d -s "$SESSION" -x 80 -y 24 \
        "python3 '$SCRIPT_DIR/fake_prompt.py' '$RESULT_FILE'"
    sleep 0.5

    # Find the target pane
    local target
    target=$(tmux list-panes -t "$SESSION" -F '#{session_name}:#{window_index}.#{pane_index}' | head -1)

    # Send the key sequence
    send_option "$target" "$option"
    sleep 0.3

    # Read result
    if [ -f "$RESULT_FILE" ]; then
        result=$(cat "$RESULT_FILE")
        if [ "$result" = "$expected" ]; then
            echo "  PASS: option $option -> selected $result"
            PASSED=$((PASSED + 1))
        else
            echo "  FAIL: option $option -> expected $expected, got $result"
            FAILED=$((FAILED + 1))
        fi
    else
        echo "  FAIL: option $option -> no result file (prompt may not have responded)"
        FAILED=$((FAILED + 1))
    fi

    tmux kill-session -t "$SESSION" 2>/dev/null || true
    sleep 0.2
}

echo "Testing select_option key sequences..."
echo ""

echo "Test 1: Select option 1 (no Down, just Enter)"
run_test 1 1

echo "Test 2: Select option 2 (one Down, then Enter)"
run_test 2 2

echo "Test 3: Select option 3 (two Downs, then Enter)"
run_test 3 3

echo ""
echo "Results: $PASSED passed, $FAILED failed"
[ "$FAILED" -eq 0 ]
