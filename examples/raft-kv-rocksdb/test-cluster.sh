#!/bin/sh

set -o errexit

cargo build

kill_all() {
    SERVICE='raft-key-value-rocks'
    if [ "$(uname)" = "Darwin" ]; then
        if pgrep -xq -- "${SERVICE}"; then
            pkill -f "${SERVICE}"
        fi
        rm -r 127.0.0.1:*.db
    else
        set +e # killall will error if finds no process to kill
        killall "${SERVICE}"
        set -e
    fi
}

rpc() {
    local uri=$1
    local body="$2"

    echo '---'" rpc(:$uri, $body)"

    {
        if [ ".$body" = "." ]; then
            curl --silent "127.0.0.1:$uri"
        else
            curl --silent "127.0.0.1:$uri" -H "Content-Type: application/json" -d "$body"
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

export RUST_LOG=debug 
bin=./target/debug/raft-key-value-rocks

echo "Killing all running raft-key-value-rocks and cleaning up old data"

kill_all
sleep 1

if ls 127.0.0.1:*.db
then
    rm -r 127.0.0.1:*.db
fi

echo "Start 3 uninitialized raft-key-value-rocks servers..."

${bin} --id 1 --http-addr 127.0.0.1:21001 --rpc-addr 127.0.0.1:22001 2>&1 > n1.log &
PID1=$!
sleep 1
echo "Server 1 started"

nohup ${bin} --id 2 --http-addr 127.0.0.1:21002 --rpc-addr 127.0.0.1:22002 > n2.log &
sleep 1
echo "Server 2 started"

nohup ${bin} --id 3 --http-addr 127.0.0.1:21003 --rpc-addr 127.0.0.1:22003 > n3.log &
sleep 1
echo "Server 3 started"
sleep 1

echo "Initialize server 1 as a single-node cluster"
sleep 2
echo
rpc 21001/cluster/init '{}'

echo "Server 1 is a leader now"

sleep 2

echo "Get metrics from the leader"
sleep 2
echo
rpc 21001/cluster/metrics
sleep 1


echo "Adding node 2 and node 3 as learners, to receive log from leader node 1"

sleep 1
echo
rpc 21001/cluster/add-learner       '[2, "127.0.0.1:21002", "127.0.0.1:22002"]'
echo "Node 2 added as leaner"
sleep 1
echo
rpc 21001/cluster/add-learner       '[3, "127.0.0.1:21003", "127.0.0.1:22003"]'
echo "Node 3 added as leaner"
sleep 1

echo "Get metrics from the leader, after adding 2 learners"
sleep 2
echo
rpc 21001/cluster/metrics
sleep 1

echo "Changing membership from [1] to 3 nodes cluster: [1, 2, 3]"
echo
rpc 21001/cluster/change-membership '[1, 2, 3]'
sleep 1
echo "Membership changed"
sleep 1

echo "Get metrics from the leader again"
sleep 1
echo
rpc 21001/cluster/metrics
sleep 1

echo "Write data on leader"
sleep 1
echo
rpc 21001/api/write '{"Set":{"key":"foo","value":"bar"}}'
sleep 1
echo "Data written"
sleep 1

echo "Read on every node, including the leader"
sleep 1
echo "Read from node 1"
echo
rpc 21001/api/read  '"foo"'
echo "Read from node 2"
echo
rpc 21002/api/read  '"foo"'
echo "Read from node 3"
echo
rpc 21003/api/read  '"foo"'

echo "Kill Node 1"
kill -9 $PID1
sleep 1

echo "Read from node 3"
echo
rpc 21003/api/read  '"foo"'
sleep 1


echo "Get metrics from node 2"
sleep 1
echo
rpc 21002/cluster/metrics
sleep 1

echo "Write data on node 2"
sleep 1
echo
rpc 21002/api/write '{"Set":{"key":"foo","value":"badger"}}'
sleep 1
echo "Data written"
sleep 1

echo "Write data on node 3"
sleep 1
echo
rpc 21003/api/write '{"Set":{"key":"foo","value":"badger"}}'
sleep 1
echo "Data written"
sleep 1


echo "Get metrics from node 2"
sleep 1
echo
rpc 21002/cluster/metrics
sleep 1


echo "Read from node 3"
echo
rpc 21003/api/read  '"foo"'
sleep 1


echo "Restart node 1"
echo

${bin} --id 1 --http-addr 127.0.0.1:21001 --rpc-addr 127.0.0.1:22001 2>&1 >> n1.log &
sleep 1
echo "Server 1 started"

echo "Read from node 1"
echo
rpc 21001/api/read  '"foo"'
sleep 1

echo "Get metrics from node 1"
sleep 1
echo
rpc 21001/cluster/metrics
sleep 1



echo "Killing all nodes in 3s..."
sleep 1
echo "Killing all nodes in 2s..."
sleep 1
echo "Killing all nodes in 1s..."
sleep 1
kill_all

rm -r 127.0.0.1:*.db