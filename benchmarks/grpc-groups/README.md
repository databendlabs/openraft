# gRPC groups

This benchmark measures Raft groups whose members run in separate processes and talk over gRPC,
in steady operation and through the crash, the hang or the partition of a node, with or without
writes. It implements the design agreed in
[#2458](https://github.com/databendlabs/openraft/issues/2458#issuecomment-7138083103).

- **Processes.** `grpc-groups run` is the driver. It starts `--nodes` processes of
  `grpc-groups node`, the same binary, on localhost, and controls them through an admin gRPC
  service. It kills node 1 in `crash` with `SIGKILL`. In `hang` it stops node 1 with `SIGSTOP`
  and later resumes it with `SIGCONT`, and in `partition` it cuts node 1 off with an admin call
  and later heals its links with another.
- **Groups.** Each node holds one member of each of `--groups` groups. Node `g % nodes + 1`
  initializes group `g` and becomes its leader, so each node first leads the same share of the
  groups.
- **Transport.** Each node implements the `GroupRouter` of `openraft-multi` over tonic. It keeps
  one gRPC channel per peer node, which all groups share over HTTP/2, and each request carries its
  group id. A heartbeat reaches the router as one `append_entries()` call, through the default
  `RaftNetworkV2::stream_append()`, and openraft counts it as failed if no reply comes within
  `heartbeat_interval`, but lets the call finish before it sends the next heartbeat to that node.
  The same default sends a group's replication to each follower one request at a time. Each
  request is one gRPC call, unless `--batch-heartbeats` sends the heartbeats to a node together:
  see [Batching heartbeats](#batching-heartbeats).
- **Types and stores.** The protobuf messages, their conversions and the in-memory stores come
  from `examples/raft-kv-memstore-grpc`.
- **Config.** The groups use openraft's default `Config`. Flags after `--` set its fields on
  every node, for example `-- --enable-pre-vote true`.

## Running

```sh
cargo run --release --manifest-path benchmarks/grpc-groups/Cargo.toml -- run --scenario crash
```

| Option           | Default  | Meaning                                                      |
|------------------|----------|--------------------------------------------------------------|
| `--scenario`     | `steady` | What happens once the groups have settled: see below.        |
| `--nodes`        | 3        | The node processes. The fault scenarios need 3 or more.      |
| `--groups`       | 1000     | The groups.                                                  |
| `--workers`      | 4        | The worker threads of the tokio runtime of each node.        |
| `--settle-secs`  | 10       | Seconds to wait once every group has a leader.               |
| `--measure-secs` | 30       | Seconds of the `measure` window of `steady`.                 |
| `--window-secs`  | 10       | Seconds of each other window: `baseline`, `degraded` and `after`. `hang` and `partition` need 1 or more. |
| `--write-rate`   | 0        | Writes per second to each group, from the settle period on: see below. |
| `--batch-heartbeats` | off  | Send the AppendEntries requests without entries to each node in batches: see [Batching heartbeats](#batching-heartbeats). |
| `--node-heartbeat-timer` | off | Trigger the heartbeats of all groups of a node from one timer: see [Batching heartbeats](#batching-heartbeats). |

## Scenarios and phases

A run starts new node processes and creates the groups. The driver spreads the initializations
over one heartbeat interval, so that the heartbeats of different groups are not in step. Once the
leader of each group has committed its blank entry, the run waits `--settle-secs`. Without
`--write-rate`, nothing is written to the groups.

`steady` then measures one window, `measure`. The fault scenarios, `crash`, `hang` and
`partition`, run these phases, and `crash` skips the last two:

| Phase      | Starts                    | Ends                                                         |
|------------|---------------------------|--------------------------------------------------------------|
| `baseline` | after the settle period   | after `--window-secs`                                        |
| `failover` | when the driver kills node 1 with `SIGKILL`, stops it with `SIGSTOP`, or cuts it off | when each group that node 1 led has a leader that committed an entry of its term, or after 30 s |
| `degraded` |                           | after `--window-secs`, without node 1                       |
| `rejoin`   | when the driver resumes node 1 with `SIGCONT`, or heals its links | when node 1 has rejoined every group, as `rejoin_*` below defines, or after 30 s |
| `after`    |                           | after `--window-secs`, with node 1                           |

A crashed node stays down: its in-memory stores lost its vote, which Raft requires a node to keep.
A stopped node keeps its state and its connections, but runs nothing, its timers included. The
other nodes get no reply from it, and openraft keeps their requests to it in flight until it
resumes: one heartbeat from each leader, and one vote from each candidate.

A cut node keeps running, but no Raft request crosses the cut and none gets a reply, as with a
blackholed network: its router holds each request to a peer without sending it, and its server
holds each request from a peer without passing it on. The connections stay open, as they would
through a real blackhole, since tonic sends no HTTP/2 keepalive pings by default. The admin calls
are not cut. openraft keeps a held heartbeat or vote until the heal. A held request whose sender
still waits at the heal goes on, as data that TCP sends again after a blackhole.

The driver and the nodes stamp the fault, the takeovers, the resume and the rejoins with
`CLOCK_MONOTONIC`, which all processes on one host share.

With `--write-rate N`, each node writes N entries a second to each group whose member on it leads
the group at the scheduled time, from the settle period to the end of the run. The writes of all
groups take turns at even intervals, open loop: a write goes out when its time comes, whether or
not the earlier ones have committed, and its latency counts from that time, so a slow leader
shows up as latency rather than as fewer writes. Two kinds of writes are not made:

- A write whose group has no leader on a running node at its time: during `failover`, the groups
  that node 1 led get no writes until their new leader takes over.
- A write that came due more than 1 s before its writer got to it, as a client that gave up
  would. Only a writer that could not run falls this far behind: without this rule, a stopped
  node 1 would send the writes of its whole stop at the resume, to groups it no longer leads.

## Batching heartbeats

Two options, both off by default, batch the heartbeats as `examples/multi-raft-kv` does:

- `--batch-heartbeats`: the router of each node collects the AppendEntries requests without
  entries, mostly heartbeats, that arrive within 1 ms of the first one, up to 1,024 requests. It
  sends those for each target node in one `AppendBatch` call. The target node passes the
  requests to their groups at the same time, and replies once each has a result, with one result
  per request.
- `--node-heartbeat-timer`: each node turns off the heartbeat timer of each group, with
  `enable_heartbeat`, and triggers the heartbeats of all its groups from one timer every
  `heartbeat_interval`, so that the heartbeats of its leaders reach the router at about the same
  time. Without it, the timer of each leader starts when the group elects it, and the driver
  spreads the initializations over one heartbeat interval.

With batching, `rpcs/s` falls below `appends/s`, since several requests share a call. `hb_rtt_*`
times each request from its arrival at the router, so it includes the wait for its batch.

With `--node-heartbeat-timer`, the leaders of a node send their heartbeats at about the same
moments, every `heartbeat_interval`. The node samples `ack_age` every 100 ms, a multiple of the
default interval of 50 ms, so all its samples fall at the same point between two rounds, and that
point differs from run to run. The `ack_age` fields and `quorum_age_max_ms` then depend on where
that point falls: in 3 runs of `steady` at 1,000 groups, `ack_age_p50_ms` was 16.2, 22.2 and
45.8 ms with the option, and 28.9 to 29.7 ms in 6 runs without it. Compare these fields only
between runs that both have the option, or that both lack it.

[`measure_change.py`](#measuring-a-change) compares two builds, and cannot compare these options
with a build that has none. Run one build with and without them in turns instead.

## Fields

Each phase prints one line of tab-separated `name=value` fields. Every line begins with
`scenario`, `phase`, `nodes` and `groups`.

The windows, `measure`, `baseline`, `degraded` and `after`, report these over every node.
`degraded` leaves node 1 out, even a cut node 1 that keeps running. Counts and rates are summed,
and percentiles cover the samples of all nodes:

| Field | Meaning |
|---|---|
| `cpu%` | the CPU time of the nodes during the window, as a share of one core |
| `memory_mib`, `memory_kib/group` | the memory of the nodes at the end, as in idle-groups: on macOS the physical footprint, on Linux `VmRSS` plus `VmSwap` |
| `tasks`, `tasks/group` | the tasks alive in the tokio runtimes of the nodes at the end |
| `appends/s`, `rpcs/s` | AppendEntries requests that the routers sent, and the gRPC calls that carried them. They are equal unless `--batch-heartbeats` sends several requests in one call. |
| `votes/s` | Vote and PreVote requests |
| `hb_rtt_p50_us`, `hb_rtt_p99_us`, `hb_rtt_max_us` | the round trip of each AppendEntries request without entries that got a reply, timed in the sending router from the arrival of the request |
| `hb_failed` | those that got no reply: the transport failed them, or openraft dropped them `election_timeout_max` after it stopped the heartbeats to their target |
| `ack_age_p50_ms`, `ack_age_p99_ms`, `ack_age_max_ms` | `now - heartbeat[follower]` on each leader, sampled every 100 ms. `degraded` leaves node 1 out. |
| `quorum_age_max_ms` | the largest `now - last_quorum_acked` that the same samples saw. A leader refuses writes once this reaches `leader_lease`, 300 ms by default. |
| `elections` | how far the highest term of each group grew, summed over the groups; 0 while every group keeps its leader |
| `writes/s` | with `--write-rate`: the writes that committed during the window |
| `write_p50_us`, `write_p99_us`, `write_max_us` | the time from the scheduled time of each of those writes until it committed |
| `write_rejected` | the writes that failed, such as on a leader whose lease expired |

`heartbeat[follower]` holds the send time of the last request that the follower acknowledged. At
idle, `ack_age` therefore ranges from one round trip to one heartbeat interval plus a round trip.

`failover` covers the groups that node 1 led:

| Field | Meaning |
|---|---|
| `affected` | how many groups node 1 led |
| `elect_p50_ms`, `elect_p90_ms`, `elect_p99_ms`, `elect_max_ms` | from the fault until a surviving member reports `ServerState::Leader` |
| `ready_p50_ms`, `ready_p90_ms`, `ready_p99_ms`, `ready_max_ms` | until that leader commits the blank entry of its term, after which writes can commit |
| `retried` | groups that needed more than one term, as after a split vote |
| `unrecovered` | groups that had no such leader when the phase ended |
| `moved` | other groups whose leader changed; 0 is expected |

`rejoin` covers every group. Node 1 records the times itself, and only from about 500 ms after the
fault on, while it is still stopped or cut off, so that each time follows its return, the resume
or the heal:

| Field | Meaning |
|---|---|
| `stepdown_p99_ms`, `stepdown_max_ms` | from the return until node 1 stops reporting `Leader` in a group that it led at the fault |
| `rejoin_p99_ms`, `rejoin_max_ms` | until node 1 first replies to an AppendEntries request of the group without rejecting the sender's vote, or leads the group in a new term |
| `unrejoined` | groups that node 1 had not rejoined when the phase ended |
| `disrupted` | groups whose highest term grew during the phase. Each held an election, so its leader stepped down, even if it won again. |

A percentile of the failover and rejoin times is exact, by the nearest rank. A percentile of a
window is within 1.6% of the true value: it comes from a log-scale histogram.

## Expected values

- A follower campaigns once `leader_lease`, which equals `election_timeout_max` (300 ms), plus
  its election timeout (150 to 300 ms) has passed since its leader last reached it. With a
  heartbeat every 50 ms, the first survivor of a group campaigns 400 to 600 ms after the fault.
  `elect_*` values far outside that range at idle need an explanation before any comparison.
- At idle, each leader sends a heartbeat to each follower about every 50 ms. idle-groups measured
  about 35,000 AppendEntries requests per second for 1,000 groups with local followers.
- In `hang`, node 1 resumes long past the election deadline of each group it followed. With
  `enable_pre_vote` off, the default, it raises the term of most of them: in 5 runs at 1,000
  groups, `disrupted` was 628 to 666 of the 666 groups that node 1 followed, and node 1 took 303 to
  620 ms to rejoin every group. With `-- --enable-pre-vote true`, `disrupted` was 0 in 4 runs, and
  node 1 rejoined every group within 79 to 83 ms.
- In `partition`, a cut node 1 hears from no leader. With `enable_pre_vote` off, it raises its
  term in the groups it follows: in 4 runs at 1,000 groups, `disrupted` was 666, every group
  node 1 followed, and node 1 took 291 to 318 ms to rejoin every group. With
  `-- --enable-pre-vote true`, `disrupted` was 0 in 4 runs, and node 1 rejoined every group
  within 16 to 28 ms.
- With `--write-rate 10` at 1,000 groups, each of 27 windows in 12 runs committed 10,000 writes a
  second. The median write took 1.1 to 2.2 ms in 25 of them, and 20 and 58 ms in the `baseline`
  and `degraded` of one `hang` run. `write_rejected` was 0 in 25 windows. The other two rejected
  9 and 371 writes, and in each a leader's `quorum_age_max_ms` exceeded the 300 ms
  `leader_lease`, at 317 and 479 ms.
- In `degraded` at 1,000 groups, `crash` counts 18,016 tasks, `partition` about 19,356, and
  `hang` about 20,356. At 300 groups, `hang` counted nearly the same with windows of 2 s and 6 s,
  so these tasks do not pile up.
  Requests to a crashed node fail at once, but those to a stopped or cut node stay in flight,
  and hyper's HTTP/2 client runs each request in flight on a task.
  The extra 1,340 and 2,340 tasks fit about 1,000 heartbeats in flight, one per group, and one
  request per group that node 1 led, with a second task per heartbeat in `hang`, where hyper also
  runs a body that the connection cannot send. The tasks have not been counted by kind to confirm
  this.

## Measuring a change

The [`measure_change.py`](../idle-groups/scripts/measure_change.py) of idle-groups compares a
change with the code before it, with `--bench grpc-groups`. It builds this benchmark without the
change and with it, in one checkout, and runs each case with the two builds in turns, so that both
see the same machine load. A case is `RUNS SCENARIO [EXTRA ARGS]`. The extra args follow
`--scenario`, so openraft `Config` flags come last, after `--`:

```sh
benchmarks/idle-groups/scripts/measure_change.py my-change $(git diff --name-only) \
    --bench grpc-groups \
    --case '3 steady' \
    --case '3 crash -- --enable-pre-vote true'
```

The script writes each line that a run prints, one per phase, to `results/LABEL-<time>.tsv` under
`benchmarks/grpc-groups/target/measure-change/`. At the end it prints the median of each field for
each build, as a Markdown table per phase of each case. The
[idle-groups README](../idle-groups/README.md#measuring-a-change) describes its other options.

CPU includes the cost of tonic, which two builds of openraft share. A difference between two
builds comes from the change, but the absolute numbers are not openraft's alone. CPU and the
round-trip percentiles also change with the load of the machine: compare them over several runs,
and report the load average.

## Results

[results.md](results.md) has the baseline at 51592651, how far the medians of two builds of the
same code differ in a comparison of 5 rounds, and the runs with batched heartbeats at a324507f.
