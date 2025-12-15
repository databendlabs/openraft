Benchmark openraft without any actual business.

This benchmark is designed to measure the performance overhead caused by
Openraft itself.

The benchmark comes with an in-memory log store, a state machine with no data,
and an in-process network that uses function calls to simulate RPC.


## Benchmark result

| clients | put/s        | ns/op      |
| --:     | --:          | --:        |
| 256     | **1,014,000** |      985   |
|  64     |  **730,000** |    1,369   |
|   1     |     70,000   | **14,273** |

The benchmark is carried out with varying numbers of clients because:
- The `1 client` benchmark shows the average **latency** to commit each log.
- The `64 client` benchmark shows the maximum **throughput**.

The benchmark is conducted with the following settings:
- No network.
- In-memory store.
- A cluster of 3 nodes in a single process on a Mac M1-Max laptop.
- Request: empty
- Response: empty


## Run it

`make bench_cluster_of_3` in repo root folder, or in this folder:

```sh
# Run with default settings (8 workers, 256 clients, 10000 ops/client, 3 members)
cargo run --release --bin bench

# Customize parameters
cargo run --release --bin bench -- -w 32 -c 1024 -n 100000 -m 3

# With tokio-console (for async task debugging)
RUSTFLAGS="--cfg tokio_unstable" cargo run --release --bin bench --features tokio-console

# With flamegraph profiling
cargo run --release --bin bench --features flamegraph
# Then generate SVG: inferno-flamegraph flamegraph.folded > flamegraph.svg
```

### CLI Options

```
-w, --workers     Number of worker threads [default: 8]
-c, --clients     Number of client tasks [default: 256]
-n, --operations  Operations per client [default: 10000]
-m, --members     Cluster size (1, 3, or 5) [default: 3]
```
