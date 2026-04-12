# OpenRaft Turmoil Simulation Tests

This package provides a deterministic simulation environment for OpenRaft using the [Turmoil](https://github.com/tokio-rs/turmoil) framework. It is designed to detect deep protocol violations and edge cases by simulating complex network conditions like partitions, packet loss, and message delays in a controlled, repeatable manner.

## Deterministic Fuzzer

The core of this testing suite is a tick-based fuzzer located in `src/bin/fuzz.rs`. Unlike traditional integration tests that rely on real time and asynchronous events, this fuzzer controls the progression of time explicitly.

### Architecture

The fuzzer operates in a strict loop:
1. **`sim.step()`**: Advances the simulation by a single discrete "tick".
2. **Invariant Checking**: Immediately after the step, the fuzzer pulls the internal state of every node in the cluster and verifies cluster-wide invariants.

This "stop-the-world" approach ensures that even transient invariant violations—which might be missed in a concurrent environment—are captured the moment they occur.

## State Access via RaftMetrics

The fuzzer reads node state through `Raft::metrics()`, the existing watch channel that `RaftCore` updates on every state change. Two fields were added to `RaftMetrics` to support invariant checking:

* **`committed: Option<LogId>`** — the last log ID this node knows to be committed.
* **`log_id_list: LogIdList`** (behind the `metrics-logids` feature flag) — per-leader log ID tracking, enabling per-index log entry lookup for log consistency verification.

These are synchronous, non-blocking reads (just cloning the latest watched value), making them safe for high-frequency use in the simulation loop.

## Determinism Enforcement

To achieve bit-for-bit reproducible simulations, the following mechanisms were implemented:

### 1. Deterministic RNG Shim
OpenRaft normally relies on the system's global random pool for election timeouts, which is non-deterministic. We introduced a `task_local!` deterministic RNG in `openraft-rt-tokio`. 
* Each Raft node is assigned a private, seeded `SmallRng` instance.
* When OpenRaft core requests a random number via the `AsyncRuntime` trait, it pulls from this scoped source instead of the system's global pool.

### 2. Node Seeding
In `spawn_cluster`, every node is initialized with a deterministic seed derived from the simulation's root seed (`node_seed = root_seed + node_id`). This ensures that even across multiple nodes, the sequence of "random" election timeouts is predictable and repeatable.

### 3. Library Version Alignment
Turmoil depends on `rand 0.8`, while the main OpenRaft project uses `rand 0.9`. To prevent version clashing, `tests-turmoil` explicitly aliases these versions:
* **`rand` (0.8)**: Used for the Turmoil simulation engine and network chaos logic.
* **`rand_09` (0.9)**: Used for OpenRaft's internal logic and the deterministic RNG shim.

### 4. Committed-State Invariants
To avoid "False Positives" in log consistency checks, the fuzzer only validates log entries that have been **committed** by both nodes. This recognizes that uncommitted entries can naturally diverge and be overwritten during standard Raft leader transitions.

### 5. High-Frequency RPC Handling (Caveat)
OpenRaft's network implementation in this fuzzer opens a new TCP connection for nearly every RPC (heartbeats, votes, append entries). In high-chaos scenarios, this can flood the simulated network faster than nodes can `accept()` them. 
* **The Symptom:** Turmoil may panic with `server socket buffer full`.
* **The Fix:** We have explicitly set `.tcp_capacity(65536)` in the simulation builder to provide a large enough buffer for these connection bursts.

### 6. Verification
Determinism can be verified by running the simulation multiple times with the same seed. After normalizing for real-world timestamps in the logs, the execution traces should be identical.

### 6. Verifying Determinism
You can verify that the simulation is bit-for-bit deterministic by running the reproduction script:

```bash
cd tests-turmoil
./repro_determinism.sh
```

This script runs the fuzzer 10 times with the same seed, normalizes the logs (by masking timestamps), and verifies that the resulting execution traces are identical using SHA256 hashes.

## Detecting and Reproducing Violations

The fuzzer is designed to provide actionable feedback when a consensus bug is found.

### Exit Codes
If an invariant violation is detected (e.g., split-brain or log divergence), the process will terminate with **Exit Code 1**. In a successful run (or when interrupted by the user), it returns **Exit Code 0**.

### Failure Reports
When a failure occurs, the fuzzer prints a structured report:
```text
=== FAILED at iteration 42 ===
Failing seed: 1770183381681145371
Steps completed: 13619
Unique states explored: 154

Violations:
  - (state) Log mismatch at index 4: node 1 has term 1, node 5 has term 2

Reproduce with: cargo run --bin fuzz -- --seed 1770183381681145371 --iterations 1
```

### Reproduction
Consensus bugs are notoriously hard to reproduce. However, because this fuzzer is **fully deterministic**, you can recreate the exact failure scenario by using the provided seed:

```bash
cargo run --bin fuzz -- --seed <FAILING_SEED> --iterations 1
```

This will regenerate the same cluster size, the same network delays, and trigger the same node crashes at the exact same millisecond. You can then add more detailed tracing or use a debugger to inspect the system state at the moment of failure.

## Running the Fuzzer

### Prerequisites
The codebase uses modern Rust features like "let chains" which require **Rust 1.92+** or a recent nightly.

### Execution
You can run the fuzzer directly with `cargo`:

```bash
cd tests-turmoil
cargo run --bin fuzz -- --iterations 1 --steps 10000
```

### Using Docker
If your local environment has an older Rust version, use the provided Docker strategy:

```bash
docker run --rm -v "$(pwd):/app" -w /app rust:1.92-bookworm /bin/bash -c "cd tests-turmoil && cargo run --bin fuzz"
```

### Options
The state-space explorer derives environmental parameters (nodes, fail-rates, timeouts) directly from the seed to maximize coverage.

| Argument | Shorthand | Default | Description |
| :--- | :--- | :--- | :--- |
| `--seed` | `-s` | Random | The root seed. Determines all parameters for every iteration. |
| `--iterations` | `-i` | `0` (Infinite) | Number of independent simulation universes to explore. `0` runs until failure. |
| `--steps` | (none) | `100,000` | The duration (in simulated ticks/ms) of each iteration. |
| `--help` | `-h` | (none) | Displays help menu. |
