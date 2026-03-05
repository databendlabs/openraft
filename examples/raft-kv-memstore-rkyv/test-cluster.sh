#!/bin/sh

set -o errexit
set -o nounset

cargo build

kill_nodes() {
    if [ "$(uname)" = "Darwin" ]; then
        SERVICE='raft-key-value'
        if pgrep -xq -- "${SERVICE}"; then
            pkill -f "${SERVICE}"
        fi
    else
        set +e # killall errors if no process is found
        killall raft-key-value
        set -e
    fi
}

wait_for_port() {
    host="$1"
    port="$2"
    timeout_secs="${3:-30}"

    i=0
    while [ "$i" -lt "$timeout_secs" ]; do
        if command -v nc >/dev/null 2>&1; then
            if nc -z "$host" "$port" >/dev/null 2>&1; then
                return 0
            fi
        elif command -v bash >/dev/null 2>&1; then
            if bash -c "exec 3<>/dev/tcp/$host/$port" >/dev/null 2>&1; then
                return 0
            fi
        fi

        i=$((i + 1))
        sleep 1
    done

    echo "ERROR: timed out waiting for ${host}:${port}" >&2
    return 1
}

probe_socket() {
    host="$1"
    port="$2"

    echo "--- probe tcp://${host}:${port}"

    if command -v nc >/dev/null 2>&1; then
        # Open and close immediately to exercise accept/process path.
        printf '' | nc "$host" "$port" >/dev/null 2>&1 || true
    elif command -v bash >/dev/null 2>&1; then
        bash -c "exec 3<>/dev/tcp/$host/$port; exec 3>&-"
    else
        echo "WARNING: neither nc nor bash /dev/tcp available; skipping active probe"
    fi
}

start_node() {
    node_id="$1"
    port="$2"
    log_file="$3"

    nohup ./target/debug/raft-key-value --id "$node_id" --addr "127.0.0.1:${port}" > "$log_file" 2>&1 &
    wait_for_port 127.0.0.1 "$port" 30
    echo "Server ${node_id} started on 127.0.0.1:${port}"
}

export RUST_LOG=trace
export RUST_BACKTRACE=full

trap 'echo "Killing all nodes..."; kill_nodes' EXIT INT TERM

echo "Killing all running raft-key-value"
kill_nodes || true
sleep 1

echo "Start 5 raft-key-value servers (TCP wire-protocol mode)..."
start_node 1 5051 n1.log
start_node 2 5052 n2.log
start_node 3 5053 n3.log
start_node 4 5054 n4.log
start_node 5 5055 n5.log

echo
echo "Recent node logs:"
for f in n1.log n2.log n3.log n4.log n5.log; do
    echo "===== ${f} ====="
    tail -n 20 "$f" || true
    echo
done

echo "Smoke test complete"
