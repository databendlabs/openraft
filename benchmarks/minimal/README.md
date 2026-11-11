Benchmark openraft without any actual business.

This benchmark is designed to measure the performance overhead caused by
Openraft itself.

The benchmark comes with an in-memory log store, a state machine with no data,
and an in-process network that uses function calls to simulate RPC.


## Benchmark result

### Single writes (batch=1)

| clients | put/s     |
| --:     | --:       |
| 1       |    33,000 |
| 64      |   912,000 |
| 256     | 1,808,000 |
| 512     | 2,543,000 |
| 1024    | 3,006,000 |
| 4096    | 3,548,000 |

### Batch writes (batch=4)

| clients | put/s     |
| --:     | --:       |
| 4096    | 5,615,000 |

The benchmark is carried out with varying numbers of clients because:
- The `1 client` benchmark shows the average **latency** to commit each log.
- The `4096 client` benchmark shows the maximum **throughput**.
- Batch writes reduce per-operation overhead by submitting multiple entries at once.

The benchmark is conducted with the following settings:
- client-workers: 1, server-workers: 16
- No network.
- In-memory store.
- A cluster of 3 nodes in a single process on a Mac M1-Max laptop.
- Request: empty
- Response: empty

### Lease reads

These runs compare `enable_local_lease_read: false` (in-core) with `true` (local) on an Apple
M3 Max: three voters in one process, in-memory storage, an in-process network, one client worker
thread, and 16 server threads. Snapshots and runtime statistics were disabled; heartbeats and
elections stayed enabled. Each run did 1,024 warmup writes, then 1,000,000 reads with one reader
or 10,000,000 reads with 64, and included the apply wait (`--read-await-ready`). The table shows
medians of five runs per mode, in alternating order.

| readers | in-core reads/s | local reads/s | gain  |
| --:     | --:             | --:           | --:   |
| 1       |   100,805       | 9,110,300     | 90.4× |
| 64      | 2,347,080       | 9,139,761     |  3.9× |

Single-reader p50 latency fell from 9.17 µs to 0.083 µs. All measured reads succeeded. An
instrumented control counted 100,000 `GetLinearizer` messages for 100,000 in-core reads and none
for 100,000 local reads.

What these numbers do not show:

- How the local path scales. All readers share the one client worker thread, so local throughput
  is the same with 1 and 64 readers: the table measures one thread running the local check. It
  does not measure contention between reader threads on the two watch channels that each local
  read loads.
- Reads queued behind writes on a busy RaftCore. With 64 readers and eight writers paced at
  180,000 reads/s and 20,000 writes/s, both paths met the target rates and read p99 was about
  2.12 ms on both, including scheduled-arrival backlog and timer scheduling. With in-memory
  storage RaftCore is probably not busy enough to queue reads.
- The apply wait under write load. In the read-only runs in the table no write is in flight, so
  the apply wait returns at once. If the read boundary is ahead of reported applied progress,
  the wait depends on RaftCore publishing newer metrics.
- Application costs. The state machine is empty; application reads, disk I/O, and network
  transport add their own costs.

### Apply-path delays

These runs measure how the state-machine worker handles a slow log read and slow
state-machine I/O. `--apply-read-delay-us` delays the log read that loads each apply batch
(`entries_stream()`); replication reads are not delayed. `--apply-io-delay-us` delays the end of
each `apply()` call, and the client responses wait for it. Both delays occupy a blocking-pool
thread, so they keep sub-millisecond accuracy.

`make bench_apply_delays` in the repo root reproduces the whole table: it runs
`scripts/apply_delays.py`, which builds `bench`, runs every row three times under
`/usr/bin/time`, and prints the table below. `--runs N` changes the number of runs, and
`--raw FILE` saves every run as a JSON line.

Setup: Apple M3 Max, macOS 14.5, three voters in one process, in-memory storage, no network
delay, one client worker thread, 16 server threads, 1,024 warmup writes, default `Config` of this
benchmark. 256 clients did 30,000 writes per run, one client did 2,000. The table shows medians
of three runs. Memory and context switches come from `/usr/bin/time -l` for the whole process.

| clients | read µs | I/O µs | writes/s  | p50 µs | p99 µs  | apply cmds | entries/apply p50, p90 | max RSS MB | invol. ctx switches |
| --:     | --:     | --:    | --:       | --:    | --:     | --:        | --:                    | --:        | --:                 |
|     256 |       0 |      0 | 1,863,377 |    128 |     259 |      1,282 |                 1, 107 |       43.0 |               8,187 |
|     256 |     500 |      0 |     6,728 | 38,397 |  66,569 |      7,835 |                   2, 9 |       43.5 |             131,098 |
|     256 |       0 |    500 |     7,814 | 33,912 |  59,449 |      6,726 |                  3, 10 |       44.2 |             110,948 |
|     256 |     500 |    500 |     4,148 | 67,788 | 111,905 |      6,484 |                  2, 10 |       43.9 |             183,191 |
|     256 |   1,000 |      0 |     3,700 | 71,875 | 119,734 |      6,951 |                  2, 11 |       44.8 |             120,213 |
|     256 |       0 |  1,000 |     3,732 | 76,521 | 116,968 |      7,302 |                   3, 9 |       44.4 |             123,746 |
|       1 |       0 |      0 |    36,727 |     26 |      46 |      3,025 |                   1, 1 |       38.6 |              17,802 |
|       1 |     500 |      0 |     1,446 |    690 |     776 |      3,025 |                   1, 1 |       39.5 |              52,233 |
|       1 |       0 |    500 |     1,444 |    690 |     758 |      3,025 |                   1, 1 |       39.3 |              51,955 |
|       1 |     500 |    500 |       727 |  1,350 |   1,762 |      3,025 |                   1, 1 |       39.8 |              83,757 |
|       1 |   1,000 |      0 |       741 |  1,326 |   2,133 |      3,025 |                   1, 1 |       39.9 |              53,408 |
|       1 |       0 |  1,000 |       722 |  1,332 |   2,635 |      3,025 |                   1, 1 |       39.9 |              52,975 |

"apply cmds" counts the leader's `SaveCommittedAndApply` commands, warmup included.

The worker runs one apply batch at a time, so the two delays add up. 500 µs of read plus
500 µs of I/O performs about like a single 1,000 µs delay: 4,148 against 3,700 to 3,732 writes/s
with 256 clients, and 727 against 722 to 741 with one. If the read of a batch overlapped the I/O
of the previous one, each batch would cost about 500 µs.

The reference point for overlapping reads and I/O is the pair of rows with 500 µs of read and
500 µs of I/O: 4,148 writes/s and 68 ms p50 with 256 clients, 727 writes/s with one client. The
rows with a single 500 µs delay are the upper bound that full overlap could reach: 6,728 to
7,814 writes/s with 256 clients, 1.6 to 1.9 times the reference, and about 1,445 writes/s with
one client, 2 times the reference.

The medians vary between runs of the whole grid. In two runs on the same machine, the
256-client medians with delays differed by up to 23% in writes/s and up to 34% in p50 latency.
To measure a change, run `make bench_apply_delays` before and after it on the same idle machine,
rather than comparing against the numbers above.

Apply batches stay small under load: with 256 clients the median batch is 2 or 3 entries.
RaftCore sends one apply command each time the commit index advances, and the queued commands
are not merged while the worker is busy. Each command pays the full delay. With 500 µs of read
and 500 µs of I/O, the worker finishes about 750 commands per second after warmup, and a write
waits behind about 50 of them.

What these numbers do not show:

- Memory cost of the queue. Max RSS stays at 43 to 45 MB in every 256-client run; the queued
  commands are too small to show up against the fixed cost of three nodes.
- Task switching inside the runtime. The context-switch counts are whole-process OS counts, and
  each simulated delay adds a wakeup of a blocking-pool thread, so runs with both delays switch
  more than runs with one.
- The 256-client run without delays takes about 16 ms and only serves as a control.

To run one row by hand:

```sh
cargo build --release --bin bench
/usr/bin/time -l target/release/bench --latency -c 256 -n 30k \
  --apply-read-delay-us 500 --apply-io-delay-us 500
```


## Run it

`make bench_cluster_of_3` in repo root folder, or in this folder:

```sh
# Run with default settings
cargo run --release --bin bench

# Customize parameters
cargo run --release --bin bench -- --client-workers 1 --server-workers 16 -c 4k -n 20m -m 3

# Batch writes (4 entries per batch)
cargo run --release --bin bench -- -c 4k -n 20m -b 4

# With tokio-console (for async task debugging)
RUSTFLAGS="--cfg tokio_unstable" cargo run --release --bin bench --features tokio-console

# With flamegraph profiling
cargo run --release --bin bench --features flamegraph
# Then generate SVG: inferno-flamegraph flamegraph.folded > flamegraph.svg
```

### Lease reads mixed with writes

Read workers share the write workers' start time. Read-enabled runs report successful and failed read latency separately from write latency.
The `config` output includes pacing, the selected read API, the idle control delay, and Raft overrides.
Unexpected read errors stop the benchmark; `--allow-read-errors` records them explicitly for fallback controls.

For a read-heavy three-voter workload:

```sh
cargo run --release --bin bench -- --latency -c 8 -n 21600 --write-rate 10800 \
  --read-clients 64 --read-operations 179200 --read-rate 89600 \
  --raft-config '{"enable_local_lease_read":true}'
```

Compare the same build with `enable_local_lease_read` set to `false`.
Reads use `get_local_read_linearizer`, which falls back to `get_read_linearizer` when caching is disabled.
Add `--read-await-ready` to include the returned linearizer's apply wait.
For read-only runs, use `--clients 0 --operations 0` with positive read client and operation counts.
Write-only arguments and output remain unchanged when `--read-clients` is zero.
Unpaced readers yield every 64 requests so cached reads still share the client runtime fairly with writers.

To measure an expired lease, run a read-only workload with heartbeats and elections disabled in `--raft-config`,
set `--read-delay-ms` beyond `election_timeout_max`, and pass `--allow-read-errors`.
Failed attempts have their own counts, latency, and error summaries.
Build with the default `runtime-stats` feature to inspect the final core message counts, including `GetLinearizer`.

### CLI Options

```
--client-workers  Number of worker threads for client runtime [default: 1]
--server-workers  Number of worker threads for server runtime [default: 16]
-c, --clients     Number of write client tasks [default: 4096]
-n, --operations  Total write operations [default: 20000000]
-m, --members     Cluster size (1, 3, or 5) [default: 3]
-b, --batch       Batch size for writes (1=single, >1=batch) [default: 1]
--read-clients    Concurrent lease-read tasks [default: 0]
--read-operations Total read attempts [default: 0]
--read-rate       Read attempts per second, or 0 for unpaced [default: 0]
--read-await-ready Include the returned linearizer's apply wait (Linearizer::await_ready)
--allow-read-errors Report read failures separately
--read-delay-ms   Idle time after warmup, before measurement [default: 0]
--apply-read-delay-us Delay of the log read that loads each apply batch [default: 0]
--apply-io-delay-us   State-machine I/O delay of each apply batch [default: 0]
```

Numbers support underscores and unit suffixes: `1_000`, `100k`, `20m`, `1g`
