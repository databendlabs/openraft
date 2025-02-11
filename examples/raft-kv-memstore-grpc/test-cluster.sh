#!/bin/sh

set -o errexit

cargo build

kill() {
    if [ "$(uname)" = "Darwin" ]; then
        SERVICE='raft-key-value'
        if pgrep -xq -- "${SERVICE}"; then
            pkill -f "${SERVICE}"
        fi
    else
        set +e # killall will error if finds no process to kill
        killall raft-key-value
        set -e
    fi
}

rpc() {
    local port=$1
    local method=$2
    local body="$3"
    local cmd="grpcurl -plaintext -proto ./proto/app.proto -d $body -import-path ./proto localhost:$port openraftpb.AppService/$method"

    echo '---'" rpc(127.0.0.1:$port/$method, $body)"

    {
	time $cmd
    } | {
        if type jq > /dev/null 2>&1; then
            jq 'if has("data") then .data |= fromjson else . end'
        else
            cat
        fi
    }

    echo
    echo
}

export RUST_LOG=trace
export RUST_BACKTRACE=full

echo "Killing all running raft-key-value"

kill

sleep 1

echo "Start 5 uninitialized raft-key-value servers..."

nohup ./target/debug/raft-key-value --id 1 --addr 127.0.0.1:5051 > n1.log &
sleep 1
echo "Server 1 started"

nohup ./target/debug/raft-key-value --id 2 --addr 127.0.0.1:5052 > n2.log &
sleep 1
echo "Server 2 started"

nohup ./target/debug/raft-key-value --id 3 --addr 127.0.0.1:5053 > n3.log &
sleep 1
echo "Server 3 started"
sleep 1

nohup ./target/debug/raft-key-value --id 4 --addr 127.0.0.1:5054 > n4.log &
sleep 1
echo "Server 4 started"
sleep 1

nohup ./target/debug/raft-key-value --id 5 --addr 127.0.0.1:5055 > n5.log &
sleep 1
echo "Server 5 started"
sleep 1

echo "Initialize servers 1,2,3 as a 3-nodes cluster"
sleep 2
echo

rpc 5051 Init '{"nodes":[{"node_id":"1","rpc_addr":"127.0.0.1:5051"},{"node_id":"2","rpc_addr":"127.0.0.1:5052"},{"node_id":"3","rpc_addr":"127.0.0.1:5053"}]}'

echo "Server 1 is a leader now"

sleep 2

echo "Get metrics from the leader"
sleep 2
echo
rpc 5051 Metrics '{}'
sleep 1


echo "Adding node 4 and node 5 as learners, to receive log from leader node 1"

sleep 1
echo
rpc 5051 AddLearner       '{"node":{"node_id":"4","rpc_addr":"127.0.0.1:5054"}}'
echo "Node 4 added as learner"
sleep 1
echo
rpc 5051 AddLearner       '{"node":{"node_id":"5","rpc_addr":"127.0.0.1:5055"}}'
echo "Node 5 added as learner"
sleep 1

echo "Get metrics from the leader, after adding 2 learners"
sleep 2
echo
rpc 5051 Metrics '{}'
sleep 1

echo "Changing membership from [1, 2, 3] to 5 nodes cluster: [1, 2, 3, 4, 5]"
echo
rpc 5051 ChangeMembership '{"members":["1","2","3","4","5"],"retain":true}'
sleep 1
echo 'Membership changed to [1, 2, 3, 4, 5]'
sleep 1

echo "Get metrics from the leader again"
sleep 1
echo
rpc 5051 Metrics '{}'
sleep 1

echo "Write foo=zoo on node-1"
sleep 1
echo
rpc 5051 Set '{"key":"foo","value":"zoo"}'
sleep 1
echo "Data written"
sleep 1

echo "Read foo=zoo from node-2"
sleep 1
echo "Read from node 2"
echo
rpc 5052 Get '{"key":"foo"}'
echo


echo "Killing all nodes..."
kill
