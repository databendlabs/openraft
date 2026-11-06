#!/bin/sh

set -o errexit

cargo build

kill_all() {
    # Match the command line: the process name is cut to 15 or 16 characters.
    pkill -f raft-key-value-weighted || true
}

rpc() {
    local uri=$1
    local body="$2"

    echo '---'" rpc(:$uri, $body)"

    {
        if [ ".$body" = "." ]; then
            time curl --silent "127.0.0.1:$uri"
        else
            time curl --silent "127.0.0.1:$uri" -H "Content-Type: application/json" -d "$body"
        fi
    } | {
        if type jq > /dev/null 2>&1; then
            jq
        else
            cat
        fi
    }

    echo
    echo
}

export RUST_LOG=trace
export RUST_BACKTRACE=full

echo "Killing all running raft-key-value-weighted"

kill_all

sleep 1

echo "Start 4 uninitialized raft-key-value-weighted servers..."

nohup ./target/debug/raft-key-value-weighted --id 1 --api-addr 127.0.0.1:21001 --raft-addr 127.0.0.1:22001 > n1.log &
nohup ./target/debug/raft-key-value-weighted --id 2 --api-addr 127.0.0.1:21002 --raft-addr 127.0.0.1:22002 > n2.log &
nohup ./target/debug/raft-key-value-weighted --id 3 --api-addr 127.0.0.1:21003 --raft-addr 127.0.0.1:22003 > n3.log &
N3=$!
nohup ./target/debug/raft-key-value-weighted --id 4 --api-addr 127.0.0.1:21004 --raft-addr 127.0.0.1:22004 > n4.log &
N4=$!
sleep 2
echo "Servers started"

echo "Initialize node 1 alone: weight 1, write weight 1"
echo
rpc 21001/init '{"weights": {"1": 1}, "write_weight": 1}'
sleep 1

echo "Adding nodes 2, 3 and 4 as learners, to receive log from leader node 1"
echo
rpc 21001/add-learner '{"node_id": 2, "api_addr": "127.0.0.1:21002", "raft_addr": "127.0.0.1:22002"}'
rpc 21001/add-learner '{"node_id": 3, "api_addr": "127.0.0.1:21003", "raft_addr": "127.0.0.1:22003"}'
rpc 21001/add-learner '{"node_id": 4, "api_addr": "127.0.0.1:21004", "raft_addr": "127.0.0.1:22004"}'
sleep 1

echo "Make all 4 nodes voters: voter 1 weighs 2, the others 1 each; a commit needs weight 3"
echo
rpc 21001/change-membership '{"weights": {"1": 2, "2": 1, "3": 1, "4": 1}, "write_weight": 3}'
sleep 1

echo "Get metrics from the leader"
echo
rpc 21001/metrics
sleep 1

echo "Write foo=1 on the leader, and read it on every node"
echo
rpc 21001/write '{"Set":{"key":"foo","value":"1"}}'
sleep 1
rpc 21001/read '"foo"'
rpc 21002/read '"foo"'
rpc 21003/read '"foo"'
rpc 21004/read '"foo"'

echo "Stop nodes 3 and 4: voters 1 and 2 still weigh 3, which reaches the write weight"
kill "$N3" "$N4"
sleep 1

echo "Write foo=2 on the leader: it commits on voters 1 and 2"
echo
rpc 21001/write '{"Set":{"key":"foo","value":"2"}}'
sleep 1
rpc 21002/read '"foo"'

echo "Killing all nodes..."
kill_all
