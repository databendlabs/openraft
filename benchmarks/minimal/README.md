Benchmark openraft without any actual business.

This benchmark is designed to measure the performance overhead caused by
Openraft itself.

The benchmark comes with an in-memory log store, a state machine with no data,
and an in-process network that uses function calls to simulate RPC.

The network replicates like a streaming transport: it passes the leader's stream of
AppendEntries requests to the follower's `Raft::stream_append()`, so several requests are in
flight at once. Before, the bench used the default `stream_append_sequential()`, which sends one
request per round trip; the tables below say which ones were measured that way.


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
- Sequential replication path (`stream_append_sequential()`), which the bench used then.

### Lease reads

These runs compare `enable_local_lease_read: false` (in-core) with `true` (local) on an Apple
M3 Max: three voters in one process, in-memory storage, an in-process network, one client worker
thread, and 16 server threads. Snapshots and runtime statistics were disabled; heartbeats and
elections stayed enabled. Replication used the sequential path, which the bench used then.
Each run did 1,024 warmup writes, then 1,000,000 reads with one reader or 10,000,000 reads with
64, and included the apply wait (`--read-await-ready`). The table shows medians of five runs per
mode, in alternating order.

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

These runs measure how the state-machine worker handles a slow log read, slow state-machine
I/O, and state-machine work per entry. Each delay column models a different cost:

- "read µs", `--apply-read-delay-us`: paid once per log read that loads an apply batch
  (`entries_stream()`). Replication reads are not delayed.
- "I/O µs", `--apply-io-delay-us`: paid once per `apply()` call, like an fsync or a round trip.
- "entry µs", `--apply-entry-delay-us`: paid once per applied entry, like writing each entry to
  its own key or running business logic per entry. An `apply()` call of n entries waits n times
  this delay, in one sleep together with the I/O delay.

The client responses of an `apply()` call wait for its I/O and entry delays. All delays occupy a
blocking-pool thread, so they keep sub-millisecond accuracy.

`make bench_apply_delays` in the repo root reproduces the whole table: it runs
`scripts/apply_delays.py`, which builds `bench`, runs every row three times under
`/usr/bin/time`, and prints the table below. `--runs N` changes the number of runs, and
`--raw FILE` saves every run as a JSON line.

The current grid requests 29,952 writes for 256 clients, matching the writes actually completed
in the historical 30,000-request runs below. This count also permits exact comparisons with
older binaries. Current batch statistics come from structured JSON and exclude bootstrap
warmup writes; the historical table includes them.

Setup: Apple M3 Max, macOS 14.5, three voters in one process, in-memory storage, no network
delay, one client worker thread, 16 server threads, 1,024 warmup writes, default `Config` of this
benchmark, pipelined replication path. 256 clients did 30,000 writes per run, one client did
2,000. The table shows medians of five runs (`--runs 5`), taken while other processes kept the
load average between 4 and 5. Memory and context switches come from `/usr/bin/time -l` for
the whole process.

| clients | read µs | I/O µs | entry µs | writes/s  | p50 µs | p99 µs | apply cmds | entries/apply p50, p90 | max RSS MB | invol. ctx switches |
| --:     | --:     | --:    | --:      | --:       | --:    | --:    | --:        | --:                    | --:        | --:                 |
|     256 |       0 |      0 |        0 | 1,793,121 |    135 |    281 |      1,290 |                 1, 115 |       40.8 |               7,749 |
|     256 |     500 |      0 |        0 |   151,372 |  1,620 |  2,610 |      1,326 |                 1, 102 |       42.7 |              21,991 |
|     256 |       0 |    500 |        0 |   166,812 |  1,553 |  1,613 |      1,376 |                 1, 110 |       42.5 |              22,362 |
|     256 |     500 |    500 |        0 |   107,932 |  2,345 |  3,217 |      1,409 |                 1, 119 |       42.3 |              35,124 |
|     256 |   1,000 |      0 |        0 |    80,631 |  3,153 |  4,833 |      1,343 |                 1, 100 |       42.4 |              22,701 |
|     256 |       0 |  1,000 |        0 |    83,307 |  3,094 |  3,285 |      1,384 |                 1, 110 |       42.6 |              22,216 |
|     256 |       0 |      0 |       10 |    68,293 |  3,898 |  4,060 |      1,387 |                 1, 110 |       41.5 |              21,192 |
|     256 |       0 |    500 |       10 |    49,471 |  5,388 |  5,600 |      1,385 |                 1, 110 |       44.0 |              21,960 |
|       1 |       0 |      0 |        0 |    37,291 |     26 |     45 |      3,025 |                   1, 1 |       36.3 |              17,053 |
|       1 |     500 |      0 |        0 |     1,219 |    819 |    903 |      3,025 |                   1, 1 |       37.4 |              51,421 |
|       1 |       0 |    500 |        0 |     1,222 |    820 |    916 |      3,025 |                   1, 1 |       37.6 |              50,873 |
|       1 |     500 |    500 |        0 |       625 |  1,589 |  1,842 |      3,025 |                   1, 1 |       37.8 |              82,684 |
|       1 |   1,000 |      0 |        0 |       643 |  1,568 |  1,683 |      3,025 |                   1, 1 |       37.8 |              52,758 |
|       1 |       0 |  1,000 |        0 |       638 |  1,576 |  1,808 |      3,025 |                   1, 1 |       37.8 |              52,056 |
|       1 |       0 |      0 |       10 |    17,947 |     55 |     72 |      3,025 |                   1, 1 |       36.5 |              48,188 |
|       1 |       0 |    500 |       10 |     1,207 |    831 |    916 |      3,025 |                   1, 1 |       37.1 |              50,801 |

"apply cmds" counts the `SaveCommittedAndApply` commands the leader's RaftCore sends, warmup
included. "entries/apply" is the number of entries the state machine receives in one `apply()`
call. The worker merges the apply commands queued when it starts a batch, so one call covers one
or more commands. Both columns include the 1,024 warmup writes, which a single client sends one
at a time, so each of them is applied alone and the p50 is 1 in every row.

RaftCore sends one apply command each time the commit index advances. The worker keeps two
batches in flight: while one batch applies with one `apply()` call, the apply commands that were
queued when that call started are read as the next batch. The commands that arrive during the
call extend that next batch: when the call returns, the worker reads them too, and applies them
with it. Under load, a batch therefore costs the larger of its read and its I/O delay, plus the
read of its extension, and each delay is paid once for all the entries of the batch.

So the read no longer adds to the I/O: 500 µs of read plus 500 µs of I/O gives 107,932 writes/s
with 256 clients, against 80,631 to 83,307 for a single 1,000 µs delay. Full overlap would reach
the 166,812 writes/s of a single 500 µs I/O delay. Part of the difference is the extension read,
which overlaps nothing, and the first batch after every idle period, whose read cannot overlap
anything either. The extension read is also why a 500 µs read alone, 151,372 writes/s, is slower
than 500 µs of I/O alone. With one client nothing can overlap: only one write is in flight, so
there is never a next batch to read ahead.

A cost per entry does not shrink when batches merge: an `apply()` call of n entries pays 10·n µs
in the "entry µs" rows, so the worker cannot apply more than 100,000 entries/s, whatever the
batch size. 256 clients reach 68,293 writes/s with that delay alone, below the 166,812 writes/s
of a 500 µs per-call I/O delay. With both, a call of 110 entries, the p90, costs 1.6 ms, and
throughput falls to 49,471 writes/s. Conclusions drawn from the per-call rows, such as how much
merging saves, hold for a state machine only to the extent that its cost is per call. With one
client, the 10 µs entry delay raises p50 from 26 to 55 µs, because handing a sleep to a
blocking-pool thread costs about 19 µs more than the sleep itself.

The medians vary between runs of the whole grid. In two runs on the same machine, the
256-client medians with delays differed by up to 23% in writes/s and up to 34% in p50 latency.
To measure a change, compare it with its base revision on the same machine, rather than against
the numbers above: `make bench_compare_apply_delays BASE=<rev>`, described in "Compare two
revisions" below.

What these numbers do not show:

- Memory cost of large batches. Max RSS stays at 41 to 44 MB in every 256-client run; the
  entries of the two batches in flight are too small to show up against the fixed cost of three
  nodes.
- Task switching inside the runtime. The context-switch counts are whole-process OS counts, and
  each simulated delay adds a wakeup of a blocking-pool thread, so runs with both delays switch
  more than runs with one.
- The 256-client run without delays takes about 20 ms and only serves as a control.

To run one row by hand:

```sh
cargo build --release --bin bench
/usr/bin/time -l target/release/bench --latency -c 256 -n 30k \
  --apply-read-delay-us 500 --apply-io-delay-us 500
```

### Apply latency below saturation

In the saturated runs above every write waits in a queue anyway, so they cannot show whether
reading ahead delays a write. Reading ahead can: without it, an entry committed during
`apply(k)` was applied right after `apply(k)` returned, but once the range of batch k+1 is fixed
when `apply(k)` starts, that entry waits for all of `apply(k+1)` too. These runs pace 256 clients
with `--write-rate` well below saturation and compare three builds:

- "merge only": c880c148, which merges queued apply commands but does not read ahead.
- "read-ahead": 9f5758a4, which reads the next batch while the current one applies.
- "extension": this revision, which also extends the read-ahead batch with the commands queued
  while the current batch applied, before applying it.

Each run did twice the write rate in writes, so it lasted about two seconds. The three builds ran
back to back for every row, five rounds, at a load average of 4 to 5; the table shows medians.
All rows reached their write rate, and there was no read delay.

| I/O µs | writes/s | p50 merge only | p50 read-ahead | p50 extension | p99 merge only | p99 read-ahead | p99 extension |
| --:    | --:      | --:            | --:            | --:           | --:            | --:            | --:           |
|    500 |    2,000 |          1,711 |          1,629 |         1,797 |          3,065 |          2,984 |         3,159 |
|    500 |    5,000 |          1,745 |          1,725 |         1,727 |          3,206 |          3,200 |         3,318 |
|    500 |   10,000 |          1,807 |          1,741 |         1,786 |          3,285 |          3,218 |         3,282 |
|  1,000 |    2,000 |          3,126 |          3,066 |         3,038 |          5,201 |          5,081 |         5,399 |
|  1,000 |    5,000 |          3,046 |          3,151 |         3,160 |          5,173 |          5,221 |         5,246 |
|  1,000 |   10,000 |          3,213 |          3,188 |         3,220 |          5,256 |          5,237 |         5,335 |
|  2,000 |    2,000 |          6,231 |          7,940 |         6,118 |          8,443 |        10,991 |         8,402 |
|  2,000 |    5,000 |          6,067 |          6,228 |         6,155 |          8,223 |        10,044 |         8,305 |
|  2,000 |   10,000 |          6,121 |          6,166 |         6,142 |          8,394 |        10,031 |         8,283 |

- With 2,000 µs of I/O, reading ahead raised p99 by 1.6 to 2.5 ms, about one I/O, and p50 by up
  to 1.7 ms. The extension brings both back to the merge-only numbers.
- With 500 and 1,000 µs of I/O, the three builds stay within 11% of each other, in no
  consistent order. Probably few writes arrive while a batch is both applying and read ahead:
  the bench releases paced writes on 1 ms timer ticks, and an `apply()` this short often ends
  before the next tick.

The extension costs one read per batch that is not overlapped, when commands arrived during the
previous `apply()`. The bench charges the full `--apply-read-delay-us` for that read, although it
covers only entries just written, which a store would usually find in its cache. In
`make bench_compare_apply_delays` against 9f5758a4, the saturated rows with a read delay and no
I/O delay lost 8 to 9% of their throughput, and their p99 rose by about half; every row with an
I/O delay, including 500 µs of read plus 500 µs of I/O, stayed within run-to-run noise.

The merge-only build replicates one AppendEntries request per round trip, the other two through
`stream_append()`. Without a network delay the two paths measured alike when they were compared.
To compare a row again:

```sh
python3 benchmarks/minimal/scripts/compare.py --base c880c148 -- \
  --latency -c 256 -n 4000 --write-rate 2000 --apply-io-delay-us 2000
```

### Replication delays

These runs measure whether the leader's log read for replication is ever on the critical path,
by delaying it together with the network. `--network-delay-us` delays every AppendEntries
request and every response. It sleeps on the Tokio timer, which rounds a deadline up to the
next 1 ms tick, so a 500 µs delay takes 0.5 to 1.5 ms each way. `--replication-read-delay-us`
delays every `limited_get_log_entries()` call, which only replication issues; the apply path
reads through `entries_stream()` and is not delayed. The read delay occupies a blocking-pool
thread, so it keeps sub-millisecond accuracy.

Setup: as in "Apply-path delays", with 256 clients doing 30,000 writes per run. The table shows
medians of five rounds, each running every row once, at a load average of 10 to 11.

| network µs | repl. read µs | writes/s  | p50 µs | p99 µs |
| --:        | --:           | --:       | --:    | --:    |
|          0 |             0 | 1,267,278 |    168 |  1,259 |
|        500 |             0 |    69,303 |  3,864 |  6,253 |
|          0 |           500 |   216,859 |  1,250 |  1,443 |
|        500 |           500 |    57,440 |  4,005 |  7,644 |

- With only the read delayed, throughput falls to 216,859 writes/s. The read is the only cost,
  and the reads for one follower run one at a time, so each read's batch sets the throughput.
- With the network delayed as well, the read adds 141 µs to p50, less than the 500 µs of one
  read, and throughput keeps 83% of the network-only 69,303 writes/s. The leader reads the log
  for the next request while earlier requests are on the wire, so most of the read is hidden
  behind the round trip.

So the leader does not serialize replication reads behind the network, the way the
state-machine worker serialized its log read behind `apply()` before it read ahead (#2320). A
worker-style read-ahead is not needed on the replication path.

What these numbers do not show:

- Network bandwidth. The delay is the same for an entry-less request and one of 1,024 entries.
- Network latency finer than the timer. Every delayed hop ends on a 1 ms timer tick.
- A remote follower. Both followers share the leader's process and runtime.
- The 256-client run without delays takes about 20 ms; its runs ranged from 0.7 to 1.46 million
  writes/s, so it only serves as a control.

To run one row by hand:

```sh
cargo build --release --bin bench
target/release/bench --latency -c 256 -n 30k \
  --network-delay-us 500 --replication-read-delay-us 500
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
They use LeaseRead unless `--read-policy read-index` is given; ReadIndex reads always run in RaftCore.
Add `--read-await-ready` to include the returned linearizer's apply wait.
Add `--read-on-follower <id>` with `--read-query` to await each linearizer and run the query on that follower,
as an application's follower read does after fetching the linearizer from the leader.
The report then also has `follower_lag`: at each applied index the follower publishes, how many entries
the leader had applied beyond it (`entries_behind`) and how much later than the leader it reached that index
(`time_behind_ns`). Negative values mean the follower was ahead.
Both nodes' applied indexes come from their metrics watch, which coalesces changes, as seen by a task
on the client runtime. `time_behind_ns` is therefore measured against the first leader value seen at
or above the index, not the moment the leader reached it, and either side can be observed late: the lag
can be off by that delay in either direction, typically a few tens of entries or about 100 µs.
For read-only runs, use `--clients 0 --operations 0` with positive read client and operation counts.
Write-only arguments and output remain unchanged when `--read-clients` is zero.
Unpaced readers yield every 64 requests so cached reads still share the client runtime fairly with writers.

To measure an expired lease, run a read-only workload with heartbeats and elections disabled in `--raft-config`,
set `--read-delay-ms` beyond `election_timeout_max`, and pass `--allow-read-errors`.
Failed attempts have their own counts, latency, and error summaries.
Build with the default `runtime-stats` feature to inspect the final core message counts, including `GetLinearizer`.

### Arrival queues and request phases

An arrival is the scheduled time when a request becomes due. Dispatch is when a worker
actually invokes that request.

Add `--phase-sample-every 1024` to sample about one in 1,024 requests. `timing.phases`
(write-only output), `write_timing.phases`, and `read.timing.phases` (mixed output)
report client delay, request service, authorization, barrier wait, query-lock wait,
and query time separately. Each histogram includes its sample count. Phase sampling
is disabled by default, so cached reads do not pay for extra phase clocks.
Sampled traces contain operation IDs and boundaries in nanoseconds since the workload epoch.
Seeded hashing spreads phase samples across clients and burst positions.

`--open-loop` generates requests independently of earlier completions. Write and read
rates must be positive for their active operation classes. `--clients` and
`--read-clients` limit executing requests; `--queue-capacity` bounds waiting requests
per class. A full queue records a rejection. The `timing` object reports offered,
admitted, completed, failed, timed-out, and rejected operations. Existing latency and
throughput fields count successful operations. Unexpected service errors are recorded
in this mode; fixed-client sessions still stop unless `--allow-read-errors` permits them.

`--arrival-pattern uniform|poisson|burst` controls arrival spacing. `--arrival-seed`
replays Poisson schedules with independent read/write streams. `--burst-size` sets
the simultaneous requests per burst. Fixed-client sessions remain the default.
`--request-timeout-ms` sets a deadline from the original scheduled arrival, including
queue delay and retries. A timed-out write has an uncertain outcome: cancellation
does not undo an accepted write.

For example, measure independent follower-read arrivals with sampled phases:

```sh
target/release/bench --latency --clients 32 --operations 20000 --write-rate 10000 \
  --read-clients 64 --read-operations 180000 --read-rate 90000 --read-query \
  --read-on-follower 1 --open-loop --queue-capacity 1024 \
  --phase-sample-every 1024 --request-timeout-ms 1000
```

Add `--arrival-noop` to run the same scheduling and admission path without measured
Raft calls. Its sampled `generator_delay`, `queue_wait`, and `client_delay` show
pacing and dispatch error. The scheduler uses Tokio timers, whose millisecond
resolution can group sub-millisecond arrivals. Measure this control before attributing
the full scheduled-arrival latency to Raft. No-op mode still creates and warms up
the cluster before the measurement.

### Comparing core, local, and relaxed reads

Run `make bench_relaxed_reads` from the repository root, or:

```sh
python3 benchmarks/minimal/scripts/relaxed_reads.py --runs 3 --raw /tmp/relaxed-reads.jsonl
```

The script builds once in release mode without runtime statistics and rotates the order of
three modes: standard in-core LeaseRead, cached local LeaseRead, and cached local LeaseRead
with `LinearizerOption::with_relaxed(true)`. All three include their apply
barrier and the same state-machine query: a write counter and its applied log ID sampled under
one read lock. The relaxed mode requires `enable_relaxed_read: true` in `--raft-config` and
`--read-relaxed --read-query`. With that setting the leader also lets followers apply only up to
its own applied log ID, so mixed runs measure that replication change as well.

The grid includes single-reader and 64-reader read-only runs, plus paced mixed workloads with
no delay, a 500 µs apply-log-read delay, and a 500 µs state-machine I/O delay. The mixed runs
schedule 90,000 reads/s and 10,000 writes/s for two seconds. Reported throughput uses total
workload elapsed time and includes any drain time. The script checks operation counts and
rejects all read failures. `--raw` preserves commands and every JSON report.

`--grid follower` runs the mixed workloads with standard reads on follower 1 instead, with and
without `enable_relaxed_read`, and adds the follower's p99 apply lag to the table. With the setting,
the leader lets followers apply only up to its own applied log ID, so this grid measures what that
bound costs follower reads.

`--grid writes` runs unpaced write-only workloads with 1, 32 and 256 clients, with no delay, a
500 µs apply-log-read delay, or a 500 µs state-machine I/O delay, with and without
`enable_relaxed_read`. With the setting, every apply result on the leader sends the new bound to
the replication streams again, and this grid measures what that costs writes.

`--grid bound` reruns the mixed workloads with the core and local modes also under
`enable_relaxed_read`. Comparing `core` with `core+bound` isolates the replication change, and
`local+bound` with `relaxed` isolates the relaxed read barrier.

`--grid read-index` runs every workload with standard and relaxed ReadIndex reads
(`--read-policy read-index`). A ReadIndex read always runs in RaftCore and waits for a quorum to
confirm leadership, so this grid shows whether skipping the apply wait still matters when each
read pays that round trip.

`--grid sweep` runs the paced mixed workload in the three modes at apply-log-read and
state-machine I/O delays of 0, 100 µs, 500 µs and 2 ms, to show how large the apply backlog must be
before relaxed reads pay off.

`--grid leader-change` transfers leadership from node 0 to node 1 one second into a three-second
mixed workload with an apply delay (`--transfer-leader-after-ms 1000`). Writers and readers follow
the new leader: a request rejected because leadership is moving waits until node 1 has taken over,
or for 1 ms, and retries. The table reports how long the election took, how long after it the first
read on node 1 finished, and the read latency in the first second after the election, without the
setting, with it, and with relaxed reads. Under the bound, followers apply less before a failover,
so the new leader has more entries to apply before its no-op.

Results of every grid, with the setup they ran on, are in
[relaxed-reads-results.md](relaxed-reads-results.md).

The no-op optimization is expected to help when committed entries wait to be applied. It cannot
remove application-side lock contention: this store holds its write lock during simulated
state-machine I/O, so even relaxed queries must wait for that lock. Read-only runs isolate API
overhead rather than the apply-backlog benefit. Results should not be compared
directly with barrier-only measurements from `--read-await-ready` without `--read-query`.

References for the optimization and its correctness requirements:

- [ReadIndex: Less Latency through Relaxed Ordering](https://github.com/drmingdrmer/consensus-essence/blob/main/src/list/raft-read-index-relaxed-order/raft-read-index-relaxed-order.md)
  explains the no-op barrier and how a read that outlives its leader is kept safe.
- [Openraft issue #1267](https://github.com/databendlabs/openraft/issues/1267) tracks the proposal
  and the contributor's step-down counterexample.
- [Openraft read protocol: opt-in relaxed leader reads](../../openraft/src/docs/protocol/read.md#opt-in-relaxed-leader-reads)
  describes the replication change and the cluster-wide deployment requirements.
- [`Config::enable_relaxed_read` documentation (source)](../../openraft/src/config/config.rs)
  describes opt-in configuration, defaults, and safety caveats.
- [`LinearizerOption::with_relaxed` API documentation (source)](../../openraft/src/raft/linearizable_read/linearizer_option.rs)
  defines the relaxed barrier and where it applies.

### Fixed measurement windows

`--measurement-seconds 30` runs a timed workload after five seconds of warmup with the same
clients, rates, and operation mix. `--warmup-seconds` changes that warmup. Count-based operation
limits are ignored in timed mode. Bootstrap `--warmup-writes` still run before the workload epoch.

```sh
target/release/bench --latency --clients 32 --operations 0 --write-rate 10000 \
  --read-clients 64 --read-operations 0 --read-rate 90000 --read-query \
  --open-loop --measurement-seconds 30 --warmup-seconds 5 --drain-seconds 5 \
  --phase-sample-every 1024 --trace-limit 4096
```

`window` reports the exact measurement boundaries. Latency and outcome counts describe requests
scheduled inside that window, including their later drain outcomes. Throughput counts successful
completions inside the window, including a warmup request that finishes there, divided by the
fixed window length. Each operation class has its own completion count. The additional
`operations_per_second_through_drain` divides completed measurement-cohort operations by the
time from measurement start through drain. Whole-workload rates include warmup and have separate names.

Write operations count entries; read operations count requests. Controlled write batches record
each acknowledged entry, including prefixes received before a deadline or stream error. Their
fixed-window rate uses each entry's acknowledgement time, including entries of warmup batches.
`request_outcomes` reports whole request/batch outcomes separately. Successful latency has one
sample per fully acknowledged request/batch; `errors` also counts requests/batches, as labelled.

`--drain-seconds` bounds the wait for accepted requests after measurement. `timing.pending`
counts operations still unanswered at the drain deadline, or unread entries after a stream error.
`unanswered_invoked` and
`unanswered_not_invoked` distinguish a service call from an expired queued request. A write
whose service call began can have an uncertain backend outcome. Timed runs stop their runtimes
after that bounded drain, including when a blocking simulated I/O remains active.

Open-loop arrivals that the generator releases after measurement ends become explicit `missed`
outcomes. They remain in the offered count and are never admitted. A generator that cannot
account its schedule before the drain deadline fails the run. `monitoring.client_backlog` is
a bounded series of approximate live counters; final outcome counts are exact.

Latency histograms retain exact counts and means. Percentiles use bucket upper bounds with
at most 1/64 relative bucket width; values below 128 ns are exact. `memory` reports allocated
histogram storage and retained trace payloads. It distinguishes the aggregate report from an
upper bound on concurrent worker storage. `--trace-limit` caps raw traces across workers;
phase histograms retain every sampled phase and report dropped raw traces separately.

Timed runs collect structured `apply_metrics` per node. `--measure-apply` enables the same
diagnostics in count-based runs. Batch histograms select apply calls by their completion time,
so a batch that begins during warmup and finishes during measurement is included.
`source_to_recorder_callback_ns` ends at RaftCore's recorder callback before watch publication.
`source_to_metrics_watch_observed_ns` ends when a task on the server runtime observes the
metrics watch; it includes observer scheduling delay. Both exclude warmup source completions.
Follower lag retains bounded recent watch observations and reports omitted nonoverlapping samples.

For read/apply diagnosis, build with `--no-default-features --features apply-pipeline` and pass
`--measure-apply-pipeline`. This separate opt-in feature compiles out all read/apply span hooks
and fields from ordinary performance builds. Per-node `apply_pipeline` reports match actual
consumed entry ranges to source reads, retaining later and unread suffixes. They validate
entry order and counts, measure all-read/store-apply overlap, and retain bounded recent
traces with explicit drop counts. Nodes share a separate `--trace-limit` budget for these spans.
Histograms select spans by their finish time; window-time
totals clip completed reads at both boundaries. Original source timestamps are reconciled
with independent reader/applier watermarks. Unresolved spans and metadata overflow keep
derived overlap unavailable; retained apply history alone does not make coverage incomplete.

Buffer observations count source-materialized entries awaiting range attribution, including
an active apply until its exit. Their entry and referenced-payload peaks cover all phases;
they do not measure channel queue occupancy, speculative credit, or actual allocation.
Range-derived buffer observations become unavailable if their metadata retention overflows.
Store boundaries exclude channel delivery and unpolled physical I/O completion.
The observers can alter scheduling, so their runs remain diagnostic.
Use the same frozen diagnostic harness and feature set across historical core revisions.
The shared comparison scripts extract multi-read apply frequency, run-level source-read latency,
and unoverlapped read time after checking every node's accounting. Full observations remain in
each saved report. An unfinished read keeps its window-time total unavailable.

Resource samples cover the process at measurement boundaries before JSON reporting. CPU per
operation uses successful window completions. RSS remains a process high-water mark since startup.
Retained-log and unapplied-entry byte estimates count inline entry metadata; actual worker
prefetch allocation is reported as unavailable. Runtime command counters use snapshots requested
at the boundaries and expose their observation times; measurement-only apply counts come from
the source observer.

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
--read-query      Include the apply wait and a coherent state-machine counter query
--read-relaxed    Request a relaxed linearizer before the query (requires --read-query)
--read-policy     How the leader confirms leadership for each read: lease or read-index [default: lease]
--read-on-follower Await the barrier and query on this follower; report its apply lag (requires --read-query)
--allow-read-errors Report read failures separately
--read-delay-ms   Idle time after warmup, before measurement [default: 0]
--transfer-leader-after-ms Move leadership from node 0 to node 1 after this delay; clients follow it
--apply-read-delay-us Delay of the log read that loads each apply batch [default: 0]
--apply-io-delay-us   State-machine I/O delay of each apply batch [default: 0]
--network-delay-us    One-way delay of every AppendEntries request and response [default: 0]
--replication-read-delay-us Delay of every replication log read [default: 0]
--apply-entry-delay-us State-machine delay of each applied entry [default: 0]
```

Numbers support underscores and unit suffixes: `1_000`, `100k`, `20m`, `1g`

### Application and transport profiles

Build with `--features realistic-profiles` to use `--profile`, which replaces empty commands
with versioned KV writes and coherent key queries. Builds without that feature reject `--profile`
before starting and retain the original zero-sized, drop-free request and `{}` wire encoding.
Reports identify compiled features; the empty overhead control uses a build without
`realistic-profiles`. Entry and inline-batch layouts are unchanged in both builds.
Use payload sizes `0`, `256`, `4096`, and `65536` in separate runs. Each logical request
allocates its own value; direct calls share its `Arc`, while serialized links decode independent
copies on followers. `--profile-keys` bounds the keyspace and `--hot-key-percent` directs
that percentage of requests to key zero. Profiles with readers require `--read-query`.

```sh
# Resolve once if no captured benchmark Cargo.lock is available.
cargo generate-lockfile --manifest-path benchmarks/minimal/Cargo.toml
CARGO_TARGET_DIR=target cargo build --release --locked --no-default-features \
  --manifest-path benchmarks/minimal/Cargo.toml --features realistic-profiles --bin bench
target/release/bench --profile --payload-bytes 4096 --profile-keys 1024 \
  --clients 16 --operations 0 --write-rate 1000 --read-clients 32 --read-operations 0 \
  --read-rate 9000 --read-query --read-on-follower 1 --open-loop \
  --warmup-seconds 2 --measurement-seconds 10 --drain-seconds 5 \
  --cache-entries 4096 --cache-miss-us 500 --log-read-bytes-per-second 100000000 \
  --apply-io-delay-us 100 --apply-bytes-per-second 100000000 --cpu-rounds 4 \
  --concurrent-queries --serialized-transport --network-delay-us 100 \
  --network-jitter-us 100 --link-bytes-per-second 100000000 --link-capacity 64
```

Recent appended log entries hit the configured tail cache. An older entry makes a read pay
one miss cost. Apply and replication share each node's configured cache-miss byte bandwidth;
cache hits retain logical-byte counts and skip device transfer cost.
The byte estimate includes inline entry metadata and application value bytes. Transaction
cost combines `--apply-io-delay-us`, `--apply-entry-delay-us`, and byte transfer time.
`--cpu-rounds` performs actual deterministic mixing over keys and values on the apply worker.
The default profile holds the query lock during this work. `--concurrent-queries` stages only
the incoming batch and publishes values and the applied log ID together after work finishes.
Write responses and the apply-completion observer follow publication and lock release.
`--follower-delay-us` adds asymmetric log-read and transaction delay on follower 1.

Serialized transport covers append streams, votes, pre-votes, transfer, snapshots, and their
responses. Each directed link shares one bandwidth reservation and a bounded admission queue.
Ordered append streams retain the underlying pipelined protocol. Tokio timers model link delays;
blocking-pool sleeps model store waits. The profile report retains bounded requested/actual
store-delay observations, including blocking-pool submission and wakeup. Compare their actual
distributions under load before interpreting a configured microsecond cost. Link reports include
serialized bytes, peak admitted messages, and encoded-buffer allocation; their memory scope
excludes original, decoded, and waiting payloads.

`profile.history` checks each read against writes acknowledged before its actual invocation.
Sampled histories include logical IDs, full acknowledged log IDs, retries, actual invocation,
completion, and uncertain batch suffixes. `--history-every` and `--history-capacity` bound raw
history storage independently of the keyspace. `--duplicate-capacity` bounds recent logical-ID
observations per state machine; eviction makes older duplicate coverage unknown. Repeated Set
commands are detected within that window and still apply. Final checks cover acknowledgement
floors and, after drain, exact values across nodes. Unfinished or locked nodes report incomplete
coverage. Arrival no-op calibration records no application history. Normal Raft ticks, heartbeats,
elections, and snapshots remain configurable with `--raft-config`.

The `profile` binary measures payload-bearing snapshot allocations after setup writes drain
and the Raft node stops. This command uses 16,384 unique writes, 4,096 keys, and five repetitions:

```sh
CARGO_TARGET_DIR=target cargo build --release --locked --no-default-features \
  --manifest-path benchmarks/minimal/Cargo.toml --features realistic-profiles --bin profile
for payload in 0 4096; do
  target/release/profile --snapshot-allocations --profile --payload-bytes "$payload" \
    --profile-keys 4096 --operations 16384 --members 1 --clients 1 --snapshots 5
done
```

Each repetition measures the whole snapshot build, outgoing snapshot retrieval, and a separate
encoded-Vec clone. Outputs remain alive at each measurement boundary; validation, drops, and
JSON reports follow it. Reports distinguish requested allocation bytes, phase live-byte peaks,
encoded length/capacity, and retained live bytes after dropping outputs. The first build creates
the cached snapshot; later builds replace it. Timings are wall time, and memory counters cover
requested Rust allocations rather than RSS or allocator internals.

With `realistic-profiles`, snapshots compact the encoded Vec before sharing immutable bytes
between the cache and outgoing cursors. Each cursor keeps its own position and copies bytes on
mutation. Direct transfers share these bytes; serialized transfers decode independent buffers
with the same wire format. Builds include compaction cost; outgoing retrieval copies metadata.

### Durable processes and HTTP clients

Build the dedicated durable node and start three owned processes with a paced HTTP workload:

```sh
cargo build --release --manifest-path benchmarks/durable/Cargo.toml
python3 benchmarks/minimal/scripts/process_profile.py --backend wal \
  --output /tmp/openraft-wal-profile --seconds 30 --read-on-follower 2
```

For persistent state-machine checkpoints, build the same crate with `--features rocks` and
select `--backend rocks`. The harness reuses example stores, handlers, and RPC servers;
benchmark probes and fault controls live under `benchmarks/durable`. Both profiles use pooled client connections, the examples' unary
sequential replication adapter, and ReadIndex authorization. Follower reads add a real HTTP
`/get_linearizer` exchange with the leader; per-route body counters expose that traffic.
These measurements have a different transport and read policy from the in-process controls.
The WAL profile syncs its log and rebuilds its volatile state machine from retained logs after
restart. Its snapshots are disabled. RocksDB uses durable logs and persistent state-machine
transactions; its snapshot scenario installs and restarts from a checkpoint.

`--scenario transfer|crash|partition|restart|snapshot` injects one event during measurement.
Crash kills and restarts the leader; restart targets a follower. Partition blocks all outgoing
Raft RPCs, including in-flight snapshots, in both directions around one node. Application HTTP
and follower authorization remain reachable; this is explicitly a Raft-only partition.
The snapshot scenario requires RocksDB, stops a follower until the leader has purged beyond
it, then requires checkpoint installation and another restart. It checks a unique marker and
its applied boundary while Raft repair is blocked, before healing the cluster. Reports retain faults, errors,
retry attempts, queue peaks, fixed-window outcomes, uncertain writes, and final value checks.
A separate repeated-CAS control records two acknowledged proposals and exactly one successful
mutation. Lost Set responses can conceal duplicate proposals; the driver reports that limit.

For separate hosts, start each durable benchmark node with `--benchmark-control-addr` and pass `--attach`
a JSON manifest: `{"nodes":[{"id":1,"api_addr":"host:41001","raft_addr":"host:42001",
"control_addr":"host:43001"}, ...]}`. The driver attaches to an initialized cluster and
uses explicit numeric IPv4/IPv6 endpoints to enforce deadlines without blocking DNS.
Attach validates server-reported storage metadata and rejects local binary/port/snapshot overrides.
Process crash/restart and snapshot scenarios require owned local
processes. `--output` must be new; commands, resolved Raft configuration, executable hash,
raw bounded histories, reports, and owned-node data/logs remain there after failure.

### Reproducible comparisons

`apply_delays.py`, `relaxed_reads.py`, and `compare.py` share workload validation,
artifact storage, and build caching. They default to five rounds and performance builds
without `runtime-stats`. Scenario order rotates between rounds; each scenario also rotates
its mode order. Comparison tables show per-round paired changes with a seeded bootstrap
95% interval. Run-level p99 values remain separate observations. Overlapping ranges describe
variation and do not establish that a change is noise. Fewer than five rounds, sampled
compiler activity, or changing runtime inputs make a group diagnostic only.

```sh
python3 benchmarks/minimal/scripts/relaxed_reads.py --grid follower \
  --scenario mixed/log-read-500us --seconds 30 --warmup-seconds 5 \
  --clients 32 --read-clients 256 --write-rate 10000 --read-rate 90000 \
  --client-workers 2 --server-workers 16 --seed 7 --output /tmp/read-pairs
python3 benchmarks/minimal/scripts/compare.py --base HEAD~1 --apply-delays \
  --scenario clients=256/read=500/io=500/entry=0 --runs 5
```

Omit `--seconds` for exact-count controls. The historical apply grid uses 29,952 writes
with 256 clients so old binaries execute the requested count exactly. Counts, rates, and
client overrides affect only operation classes active in a scenario. Structured outcomes
must conserve all offered work; intentional overload retains rejected, timed-out, and pending
operations. Legacy reports without outcome accounting must complete the exact requested count.
Outcome tables show completion coverage and retained latency sample counts. Overload capacity
comparisons remain available; percentiles describe completed requests only and incomplete
coverage makes latency comparisons conditional.
Read grids enable ticking, heartbeats, and elections, including lease renewal for long
read-only runs. Explicit disabled-heartbeat controls require `--lease-expiry-control`.
Existing tables above describe their original revisions and settings.

Use `--profile` with `--payload-bytes`, `--profile-keys`, `--hot-key-percent`, and storage/link
controls to select the realistic workload. The scripts compile `realistic-profiles` when
needed. `--diagnostics` runs matching `runtime-stats` companions in a separate group with
the same lockfile. Optional apply observers run only in diagnostics and only when every
compared binary supports them.
For a custom `compare.py` command, put workload controls after `--`; outer controls apply
to generated grids. For example, `-- --latency -c 256 -n 30000 --arrival-seed 7`.

Every invocation saves a versioned manifest, complete run JSON, commands, stdout, stderr,
host samples, failures, and summaries before rendering tables. `--raw` adds a full-record
JSONL copy. `--timeout` and `--build-timeout` bound owned command groups. Active system load
is recorded; saturation alone does not invalidate a run. Compiler observations are sampled
and cannot prove that no short background build occurred.

The default cache is `$XDG_CACHE_HOME/openraft-bench` or `~/.cache/openraft-bench`, outside
disposable worktrees. Override it with `--cache-dir`. Each immutable binary entry retains
its source snapshot, actual lockfile, build logs, compiler, features, flags/configuration,
and hashes. Ignored lockfiles are captured or generated once, then builds use `--locked`.
Supply `--lockfile`, or `compare.py`'s `--base-lockfile` and `--target-lockfile`, to reuse a
specific dependency graph. Toolchains resolve independently in each source's crate directory;
explicit per-reference overrides are also available. Cache reads verify the stored binary
and source inputs. Cargo discovers configuration in the source snapshot, cache ancestors,
and `CARGO_HOME`; it does not inherit the original checkout's parent-directory configuration.
`--binary-manifest` reuses a verified entry; comparisons accept separate
base/target manifests. The scripts require Python 3.11 or newer.

The process driver builds its dedicated node into the same cache unless `--binary` or
`--binary-manifest` selects one. An explicit standalone binary has an unknown build-input
history. `--server-workers` sets Tokio workers per owned node and runtime metadata verifies
the effective count. Reports distinguish these workers from Python client threads. Attached
runs verify node IDs, distinct numeric endpoints, storage metadata, and empty initial fault
controls. Older attached nodes without runtime metadata report an unknown worker allocation.
