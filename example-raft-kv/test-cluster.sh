#!/bin/sh

set -o errexit

cargo build

rpc() {
    local uri=$1
    local body="$2"

    echo '---'" rpc(:$uri, $body)"

    {
        if [ ".$body" == "." ]; then
            curl --silent "127.0.0.1:$uri"
        else
            curl --silent "127.0.0.1:$uri" -H "Content-Type: application/json" -d "$body"
        fi
    } | {
        if which -s jq; then
            jq
        else
            cat
        fi
    }

    echo
    echo
}

export RUST_LOG=debug 

echo "=== Kill all running raftkv"

killall raftkv
sleep 1

echo "=== Start 3 uninitialized raftkv servers: 1, 2, 3"

nohup ./target/debug/raftkv  --id 1 --http-addr 127.0.0.1:21001 > n1.log &
nohup ./target/debug/raftkv  --id 2 --http-addr 127.0.0.1:21002 > n2.log &
nohup ./target/debug/raftkv  --id 3 --http-addr 127.0.0.1:21003 > n3.log &
sleep 1

echo "=== Initialize node-1 as a single-node cluster"

rpc 21001/init '{}'
sleep 0.2
rpc 21001/metrics


echo "=== Add 3 node addresses so that RaftNetwork is able to find peers by id"

rpc 21001/write   '{"AddNode":{"id":1,"addr":"127.0.0.1:21001"}}'
rpc 21001/write   '{"AddNode":{"id":2,"addr":"127.0.0.1:21002"}}'
rpc 21001/write   '{"AddNode":{"id":3,"addr":"127.0.0.1:21003"}}'

echo "=== List known nodes in clusters"

rpc 21001/list-nodes

echo "=== Add node-2 and node-2 as Learners, to receive log from leader node-1"

rpc 21001/add-learner       '2'
rpc 21001/add-learner       '3'

echo "=== Change membership from [1] to 3 nodes cluster: [1, 2, 3]"

rpc 21001/change-membership '[1, 2, 3]'
rpc 21001/metrics

echo "=== Write on leader and read on every node"

rpc 21001/write '{"Set":{"key":"foo","value":"bar"}}'
sleep 0.1

rpc 21001/read  '"foo"'
rpc 21002/read  '"foo"'
rpc 21003/read  '"foo"'
