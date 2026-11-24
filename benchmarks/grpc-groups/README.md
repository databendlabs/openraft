# gRPC groups

This benchmark measures Raft groups whose members run in separate processes and talk over gRPC,
in steady operation and through the crash of a node. It implements the first part of the design
agreed in [#2458](https://github.com/databendlabs/openraft/issues/2458#issuecomment-7138083103).

- **Processes.** `grpc-groups run` is the driver. It starts `--nodes` processes of
  `grpc-groups node`, the same binary, on localhost, controls them through an admin gRPC service,
  and kills node 1 in `crash` with `SIGKILL`.
- **Groups.** Each node holds one member of each of `--groups` groups. Node `g % nodes + 1`
  initializes group `g` and becomes its leader, so each node first leads the same share of the
  groups.
- **Transport.** Each node implements the `GroupRouter` of `openraft-multi` over tonic. It keeps
  one gRPC channel per peer node, which all groups share over HTTP/2, and each request carries its
  group id. A heartbeat reaches the router as one `append_entries()` call, through the default
  `RaftNetworkV2::stream_append()`, and openraft drops it if no reply comes within
  `heartbeat_interval`. The same default sends a group's replication to each follower one
  request at a time.
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
| `--nodes`        | 3        | The node processes. `crash` needs 3 or more.                 |
| `--groups`       | 1000     | The groups.                                                  |
| `--workers`      | 4        | The worker threads of the tokio runtime of each node.        |
| `--settle-secs`  | 10       | Seconds to wait once every group has a leader.               |
| `--measure-secs` | 30       | Seconds of the `measure` window of `steady`.                 |
| `--window-secs`  | 10       | Seconds of each window of `crash`: `baseline` and `degraded`. |

## Scenarios and phases

A run starts new node processes and creates the groups. The driver spreads the initializations
over one heartbeat interval, so that the heartbeats of different groups are not in step. Once the
leader of each group has committed its blank entry, the run waits `--settle-secs`. Nothing is
written to the groups.

`steady` then measures one window, `measure`. `crash` runs these phases:

| Phase      | Starts                    | Ends                                                         |
|------------|---------------------------|--------------------------------------------------------------|
| `baseline` | after the settle period   | after `--window-secs`                                        |
| `failover` | when the driver kills node 1 with `SIGKILL` | when each group that node 1 led has a leader that committed an entry of its term, or after 30 s |
| `degraded` |                           | after `--window-secs`, without node 1                       |

A crashed node stays down: its in-memory stores lost its vote, which Raft requires a node to keep.

The driver and the nodes stamp the fault and the takeovers with `CLOCK_MONOTONIC`, which all
processes on one host share.

## Fields

Each phase prints one line of tab-separated `name=value` fields. Every line begins with
`scenario`, `phase`, `nodes` and `groups`.

The windows, `measure`, `baseline` and `degraded`, report these over the node processes that are
running. Counts and rates are summed, and percentiles cover the samples of all nodes:

| Field | Meaning |
|---|---|
| `cpu%` | the CPU time of the nodes during the window, as a share of one core |
| `memory_mib`, `memory_kib/group` | the memory of the nodes at the end, as in idle-groups: on macOS the physical footprint, on Linux `VmRSS` plus `VmSwap` |
| `tasks`, `tasks/group` | the tasks alive in the tokio runtimes of the nodes at the end |
| `appends/s`, `rpcs/s` | AppendEntries requests that the routers sent, and the gRPC calls that carried them. They are equal until heartbeats are batched. |
| `votes/s` | Vote and PreVote requests |
| `hb_rtt_p50_us`, `hb_rtt_p99_us`, `hb_rtt_max_us` | the round trip of each AppendEntries request without entries that got a reply, timed in the sending router |
| `hb_failed` | those that got no reply: openraft dropped them at their timeout, `heartbeat_interval`, or the transport failed them |
| `ack_age_p50_ms`, `ack_age_p99_ms`, `ack_age_max_ms` | `now - heartbeat[follower]` on each leader, sampled every 100 ms. `degraded` leaves node 1 out. |
| `quorum_age_max_ms` | the largest `now - last_quorum_acked` that the same samples saw. A leader refuses writes once this reaches `leader_lease`, 300 ms by default. |
| `elections` | how far the highest term of each group grew, summed over the groups; 0 while every group keeps its leader |

`heartbeat[follower]` holds the send time of the last request that the follower acknowledged. At
idle, `ack_age` therefore ranges from one round trip to one heartbeat interval plus a round trip.

`failover` covers the groups that node 1 led:

| Field | Meaning |
|---|---|
| `affected` | how many groups node 1 led |
| `elect_p50_ms`, `elect_p90_ms`, `elect_p99_ms`, `elect_max_ms` | from the kill until a surviving member reports `ServerState::Leader` |
| `ready_p50_ms`, `ready_p90_ms`, `ready_p99_ms`, `ready_max_ms` | until that leader commits the blank entry of its term, after which writes can commit |
| `retried` | groups that needed more than one term, as after a split vote |
| `unrecovered` | groups that had no such leader when the phase ended |
| `moved` | other groups whose leader changed; 0 is expected |

A percentile of the failover times is exact, by the nearest rank. A percentile of a window is
within 1.6% of the true value: it comes from a log-scale histogram.

## Expected values

- A follower campaigns once `leader_lease`, which equals `election_timeout_max` (300 ms), plus
  its election timeout (150 to 300 ms) has passed since its leader last reached it. With a
  heartbeat every 50 ms, the first survivor of a group campaigns 400 to 600 ms after the kill.
  `elect_*` values far outside that range at idle need an explanation before any comparison.
- At idle, each leader sends a heartbeat to each follower about every 50 ms. idle-groups measured
  about 35,000 AppendEntries requests per second for 1,000 groups with local followers.

CPU includes the cost of tonic, which two builds of openraft share. A difference between two
builds comes from the change, but the absolute numbers are not openraft's alone. CPU and the
round-trip percentiles also change with the load of the machine: compare them over several runs,
and report the load average.
