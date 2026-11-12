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
of three runs, taken while other processes kept the load average around 13. Memory and context
switches come from `/usr/bin/time -l` for the whole process.

| clients | read µs | I/O µs | writes/s  | p50 µs | p99 µs | apply cmds | entries/apply p50, p90 | max RSS MB | invol. ctx switches |
| --:     | --:     | --:    | --:       | --:    | --:    | --:        | --:                    | --:        | --:                 |
|     256 |       0 |      0 | 1,849,027 |    127 |    327 |      1,291 |                 1, 108 |       43.2 |               8,802 |
|     256 |     500 |      0 |   160,930 |  1,593 |  1,750 |      1,389 |                 1, 102 |       44.1 |              22,349 |
|     256 |       0 |    500 |   156,011 |  1,608 |  2,198 |      1,353 |                 1, 105 |       46.6 |              21,755 |
|     256 |     500 |    500 |    82,416 |  3,100 |  4,050 |      1,374 |                 1, 102 |       46.1 |              35,492 |
|     256 |   1,000 |      0 |    79,639 |  3,100 |  5,424 |      1,393 |                  1, 99 |       45.4 |              22,964 |
|     256 |       0 |  1,000 |    83,955 |  3,085 |  3,237 |      1,349 |                 1, 101 |       44.4 |              22,031 |
|       1 |       0 |      0 |    35,861 |     26 |     50 |      3,025 |                   1, 1 |       38.6 |              16,343 |
|       1 |     500 |      0 |     1,145 |    829 |  1,762 |      3,025 |                   1, 1 |       39.8 |              50,546 |
|       1 |       0 |    500 |     1,266 |    795 |    847 |      3,025 |                   1, 1 |       39.6 |              55,746 |
|       1 |     500 |    500 |       622 |  1,586 |  1,933 |      3,025 |                   1, 1 |       39.9 |              83,433 |
|       1 |   1,000 |      0 |       635 |  1,564 |  2,066 |      3,025 |                   1, 1 |       39.8 |              53,668 |
|       1 |       0 |  1,000 |       640 |  1,562 |  1,973 |      3,025 |                   1, 1 |       39.9 |              54,084 |

"apply cmds" counts the `SaveCommittedAndApply` commands the leader's RaftCore sends, warmup
included, and "entries/apply" is the size of those commands. The worker merges queued commands
before applying them, which these two columns do not show.

RaftCore sends one apply command each time the commit index advances. When the worker takes a
command, it also takes the apply commands already queued right after it and applies the whole
range with one `entries_stream()` call and one `apply()` call. Under load, the per-call delays
are then shared by every entry committed while the previous batch was applying.

The worker still runs one batch at a time, so the two delays add up. 500 µs of read plus
500 µs of I/O performs about like a single 1,000 µs delay: 82,416 against 79,639 to 83,955
writes/s with 256 clients, and 622 against 635 to 640 with one. If the read of a batch overlapped
the I/O of the previous one, each batch would cost about 500 µs.

The reference point for overlapping reads and I/O is the pair of rows with 500 µs of read and
500 µs of I/O: 82,416 writes/s and 3.1 ms p50 with 256 clients, 622 writes/s with one client. The
rows with a single 500 µs delay are the upper bound that full overlap could reach: 156,011 to
160,930 writes/s with 256 clients, about 1.9 times the reference. With one client no
cross-batch mechanism can help: only one write is in flight, so there is never a next batch to
overlap.

The medians vary between runs of the whole grid. In two runs on the same machine, the
256-client medians with delays differed by up to 23% in writes/s and up to 34% in p50 latency.
To measure a change, compare it with its base revision on the same machine, rather than against
the numbers above: `make bench_compare_apply_delays BASE=<rev>`, described in "Compare two
revisions" below.

What these numbers do not show:

- Memory cost of large batches. Max RSS stays at 43 to 47 MB in every 256-client run; the
  entries of one batch are too small to show up against the fixed cost of three nodes.
- Task switching inside the runtime. The context-switch counts are whole-process OS counts, and
  each simulated delay adds a wakeup of a blocking-pool thread, so runs with both delays switch
  more than runs with one.
- Costs that grow with the number of entries. Both simulated delays are fixed per call, so
  merging spreads them over every entry of a batch; a per-entry cost would not shrink.
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

### Compare two revisions

`scripts/compare.py` builds `bench` at two git revisions and runs the same arguments with both,
so a change can be measured against its base on one machine:

```sh
# The arguments of `make bench_write_latency`:
make bench_compare_write_latency BASE=<rev>

# The grid of `make bench_apply_delays`:
make bench_compare_apply_delays BASE=<rev>

# Any bench arguments; they must include --latency:
python3 benchmarks/minimal/scripts/compare.py --base <rev> -- --latency -c 256 -n 30k
```

The target is the current checkout, uncommitted changes included, unless `TARGET=<rev>` (or
`--target`) names a revision. `RUNS=N` (or `--runs`) sets the runs per build, 5 by default. The
two binaries run back to back for every row and swap order on every run, so a drift in machine
load affects both alike. The script prints the medians of writes/s, p50 and p99 for both builds
and their change, and marks a change with `~` when the min-max ranges of the two builds overlap.
A revision is built in a temporary git worktree and its binary is cached under
`target/compare/<commit>`.

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
