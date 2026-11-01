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
--read-await-ready Include the apply wait through ensure_linearizable
--allow-read-errors Report read failures separately
--read-delay-ms   Idle time after warmup, before measurement [default: 0]
```

Numbers support underscores and unit suffixes: `1_000`, `100k`, `20m`, `1g`
