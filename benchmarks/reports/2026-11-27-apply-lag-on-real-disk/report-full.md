# Apply lag on real disk

## Summary

- Apply lagged commit by one to two `apply()` calls on real disk, as in the in-process runs of
  [the apply-lag report](../2026-11-25-apply-lag/report-full.md). On the leader, committed to
  applied took 0.02 ms with the in-memory state machine, 2.3 to 2.5 ms with 2 ms calls
  (1.2 calls), and 17.3 to 18.4 ms with 10 ms calls (1.7 to 1.8 calls), at every write rate.
- The disk set the commit time. The leader's WAL append took 19 to 21 ms from submission to
  persisted in every row, and clients waited 23 ms for a write with the in-memory state
  machine. One `F_FULLFSYNC` takes 4.7 ms alone on this SSD, and all three nodes sync it.
- With 2 ms calls, apply was 10% of a write's time on the leader from proposal to applied. With
  10 ms calls it was 44 to 47%, and since `/write` answers after apply, 128 clients reached only
  2,540 writes/s where the faster state machines reached about 3,700.
- Starting apply at commit, as from `save_committed()`, could save at most committed to applied
  minus one call: about 0.4 ms of 25 ms with 2 ms calls, and about 8 ms of 40 ms with 10 ms
  calls.
- 25 of 45 runs had a client p99 above 0.5 s, in every row. Apply did not cause these stalls:
  they hit the rows without an apply delay about as often. In the one stall I traced, both
  followers' AppendEntries hit the leader's 375 ms RPC timeout, and syncs on this machine slow
  down to 130 to 290 ms in bursts.

## What was measured

The target is `benchmarks/durable` at `a324507f1e7ff31068312c7318b1330c164eaeb8` (`ups/main`)
with this change, run by `benchmarks/minimal/scripts/process_profile.py`: three node processes
on one machine with the WAL backend, the examples' HTTP transport and their unary sequential
replication adapter. Each node keeps its log in `log-wal`, which syncs every append with
`sync_data()`, `F_FULLFSYNC` on macOS, before it reports the entries persisted. Its state machine
is `sm-mem`, in memory. `--apply-delay-us` adds a fixed delay to every `apply()` call, on a
blocking-pool thread before the call applies its entries:

| apply delay | stands for |
| --:         | :--        |
| 0           | the in-memory store alone |
| 2 ms        | a store that does a few ms of I/O per batch |
| 10 ms       | a store that syncs each batch to this disk, about two syncs |

Each run has 128 client threads that write only, a 256-byte value to one of 64 keys, paced at
500, 2,000 or 4,000 writes/s. A run warms up for 3 s, measures for 10 s, and drains for 5 s.
The driver measures client latency from each write's scheduled arrival. After the workload it
reads each node's runtime stats:

- **persist**, **persisted→committed**, **committed→applied**, **proposed→applied**: per-entry
  stage latencies, over the node's last 1,024 log ranges (`Config::log_stage_capacity`).
  "Committed" is when `RaftCore` starts `SaveCommittedAndApply`
  for the entry, and "applied" is when it handles the state machine worker's result.
- **entries per `apply()`**: the entries of every `apply()` call since the node started,
  including the warmup.

Most log ranges held one entry, so the stage latencies cover the last 2 s of a run at 500
writes/s and the last 0.3 to 0.5 s at 2,000 writes/s or more.

## Method

- Machine: Apple M3 Max (16 cores, 128 GiB), macOS 14.5, APFS on the internal 4 TB SSD with
  417 GB free, rustc 1.96.0-nightly (2026-02-28), Python 3.12.10. Each node runs 16 Tokio worker
  threads. The nodes use the WAL example's `Config`: 500 ms heartbeats and snapshots disabled.
- Each row ran 5 times. Every round ran all rows once, in an order shuffled with seed 1127.
  Tables show medians over runs. The 1-minute load average at the start of a run ranged from
  3.7 to 12.2 (median 5.2): the machine was not idle, and other processes used its disk.
- All 45 runs passed the driver's final check: every node applied the same value for each of
  the 64 keys, at least as new as its last acknowledged write. One earlier run failed while the
  driver set the cluster up, before this change made the driver wait for the leader's first
  commit; it was run again.
- A Python loop of 300-byte appends, each followed by `F_FULLFSYNC`, took 4.7 ms per sync at
  p50 and 11.5 ms at p99 over 60 s, with one process on the disk.

## Results

The complete tables, with the followers' stages and every run, are in `results/tables.md`.

### Writes and the leader's stages

Client latency in ms from the scheduled arrival; stages in ms per entry, p50 / p99.

| apply delay | offered  | writes/s | client p50 | client p99 | runs with p99 > 0.5 s | persist     | committed→applied | proposed→applied |
| --:         | --:      | --:      | --:        | --:        | --:                   | --:         | --:               | --:              |
| 0           | 500/s    |      500 |       23.1 |       41.9 | 2 of 5                | 19.6 / 33.8 | 0.02 / 0.2        | 22.1 / 35.0      |
| 0           | 2,000/s  |    2,000 |       23.1 |       37.7 | 2 of 5                | 19.1 / 29.3 | 0.02 / 0.04       | 20.7 / 28.5      |
| 0           | 4,000/s  |    3,693 |       23.1 |      1,074 | 4 of 5                | 20.5 / 27.2 | 0.03 / 0.1        | 21.2 / 24.6      |
| 2 ms        | 500/s    |      500 |       26.2 |        260 | 2 of 5                | 20.4 / 35.9 | 2.4 / 5.0         | 24.4 / 40.9      |
| 2 ms        | 2,000/s  |    2,000 |       25.2 |        243 | 2 of 5                | 20.2 / 28.9 | 2.3 / 4.4         | 24.4 / 32.0      |
| 2 ms        | 4,000/s  |    3,681 |       25.2 |      1,074 | 3 of 5                | 19.0 / 26.4 | 2.5 / 5.0         | 23.2 / 28.2      |
| 10 ms       | 500/s    |      500 |       41.9 |        738 | 3 of 5                | 20.6 / 28.2 | 17.9 / 25.3       | 40.4 / 51.8      |
| 10 ms       | 2,000/s  |    1,900 |       44.0 |      1,040 | 3 of 5                | 19.6 / 24.5 | 18.4 / 26.3       | 39.6 / 48.8      |
| 10 ms       | 4,000/s  |    2,541 |        419 |      1,409 | 4 of 5                | 20.5 / 28.4 | 17.3 / 26.6       | 39.5 / 48.9      |

**Commit.** The leader's append took 19 to 21 ms from submission to persisted at p50 in every
row, four times one sync of a lone process. The leader's own append finished last: persisted to
committed was 0.0 ms at p50, so a follower's append had already finished. Each follower's
append took 14 ms at p50. I did not trace where the extra time goes. The three processes sync
files on one drive, and `F_FULLFSYNC` flushes the drive's whole cache, so each append likely
waits for the other nodes' syncs as well as the one in progress.

**Apply.** Committed to applied took one to two calls, in every row and at every rate, with a
run-to-run range of 2.34 to 2.54 ms for 2 ms calls. A write committed during a call waits for
the rest of that call and then for the next one, which applies it; at 2,000 writes/s the leader
applied 9 entries per call at p50 with 2 ms calls and 25 with 10 ms calls. The followers'
committed to applied was 2.4 to 2.5 ms with 2 ms calls and 11.8 to 12.0 ms with 10 ms calls.
A follower learns the commit index from the next AppendEntries, about every 14 ms, so its
commits arrive in batches, and it applied more entries per call: 29 at 2,000 writes/s.

**What apply costs a client.** `/write` answers after the entry is applied, so a client waits
for commit and apply in turn. With 10 ms calls the leader's committed to applied was 44 to 47%
of its proposed to applied, and client p50 rose from 23 to 42 ms. At 4,000 writes/s the 128
clients then saturated: each write took 48 ms of service at p50, so 128 clients could complete
about 2,660 writes/s, and the driver rejected 14,588 writes whose queue was full.

### Stalls

25 of 45 runs had a client p99 above 0.5 s, spread over every row. During a stall the clients'
writes stopped completing for about a second, the driver's queue filled, and at 4,000 writes/s
it rejected 3,200 writes in the median run. Apply did not cause them: 8 of the 15 runs without an
apply delay stalled, against 10 of the 15 with 10 ms calls.

To find the cause, I ran one more 20 s run at 500 writes/s with every write in the history and
OpenRaft's logs at `info`. Its writes stalled once, from 10.9 to 12.0 s into the run. The run's
only warnings fell inside that window by the node clock: within 0.7 s, eight AppendEntries from
the leader to both followers hit the HTTP client's timeout. That timeout is the replication
RPC's soft TTL, 3/4 of `heartbeat_interval`, so 375 ms. With one process on the disk, the sync
loop above also saw bursts of syncs of 130 to 290 ms, at 5, 23 and 40 s into the minute. So a
burst of slow syncs on the shared disk can hold a follower's append past 375 ms; the leader then
drops the request and sends the entries again. I did not trace a stall further than this.

## What these numbers do not show

- **A state machine that syncs to the same disk.** The apply delay is a sleep: it does not
  compete with the WAL for the drive. A state machine that syncs each batch would also slow the
  WAL's syncs.
- **Separate disks and hosts.** All three nodes share one SSD, one machine and its loopback
  network, and the machine ran other work. On separate disks each append would wait for fewer
  syncs.
- **Other platforms.** `F_FULLFSYNC` flushes the drive's cache on macOS; `fdatasync` on Linux
  costs a different amount.
- **Higher write rates.** The Python driver, with 128 threads and the unary HTTP replication
  adapter, is not a load generator for more than a few thousand writes/s, and these rows cannot
  show an apply backlog that grows.
- **The whole run.** Stage latencies cover each node's last 1,024 log ranges; the entries per
  `apply()` call include the warmup.
- **RocksDB.** `--apply-delay-us` applies to the WAL backend only.

## Reproduce

From the repository root:

```sh
cargo build --release --manifest-path benchmarks/durable/Cargo.toml
for delay in 0 2000 10000; do
  for rate in 500 2000 4000; do
    python3 benchmarks/minimal/scripts/process_profile.py --backend wal \
      --binary benchmarks/durable/target/release/bench-durable-node \
      --output /tmp/realdisk/d$delay-r$rate --seconds 10 --warmup 3 --drain 5 \
      --write-rate $rate --clients 128 --read-rate 0 --apply-delay-us $delay
  done
done
```

Each run writes `report.json` to its `--output` directory: client latency in
`workload.write`, and each node's role, stage latencies (`log_stages_us`, µs per entry) and
`apply()` call sizes (`apply_call_entries`) in `runtime_stats`.
