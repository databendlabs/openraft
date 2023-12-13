Benchmark openraft without any actual business.

This benchmark is designed to measure the performance overhead caused by
Openraft itself.

The benchmark comes with an in-memory log store, a state machine with no data,
and an in-process network that uses function calls to simulate RPC.


## Benchmark result

| clients | put/s        | ns/op      |
| --:     | --:          | --:        |
| 256     | **1014,000** |      985   |
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
cargo test --test benchmark --release bench_cluster_of_3 -- --ignored --nocapture
```
