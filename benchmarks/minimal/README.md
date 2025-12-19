Benchmark openraft without any actual business.

This benchmark is designed to measure the performance overhead caused by
Openraft itself.

The benchmark comes with an in-memory log store, a state machine with no data,
and an in-process network that uses function calls to simulate RPC.


## Benchmark result

| clients | put/s     |
| --:     | --:       |
| 1       |    33,000 |
| 64      |   912,000 |
| 256     | 1,808,000 |
| 512     | 2,543,000 |
| 1024    | 3,006,000 |
| 4096    | 3,087,000 |

The benchmark is carried out with varying numbers of clients because:
- The `1 client` benchmark shows the average **latency** to commit each log.
- The `4096 client` benchmark shows the maximum **throughput**.

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

# With tokio-console (for async task debugging)
RUSTFLAGS="--cfg tokio_unstable" cargo run --release --bin bench --features tokio-console

# With flamegraph profiling
cargo run --release --bin bench --features flamegraph
# Then generate SVG: inferno-flamegraph flamegraph.folded > flamegraph.svg
```

### CLI Options

```
--client-workers  Number of worker threads for client runtime [default: 2]
--server-workers  Number of worker threads for server runtime [default: 16]
-c, --clients     Number of client tasks [default: 4096]
-n, --operations  Total operations across all clients [default: 20000000]
-m, --members     Cluster size (1, 3, or 5) [default: 3]
```

Numbers support underscores and unit suffixes: `1_000`, `100k`, `20m`, `1g`
