# OpenRaft Turmoil Simulation Tests

This package provides a deterministic simulation environment for OpenRaft using the [Turmoil](https://github.com/tokio-rs/turmoil) framework. It is designed to detect deep protocol violations and edge cases by simulating complex network conditions like partitions, packet loss, and message delays in a controlled, repeatable manner.

## Deterministic Fuzzer

The core of this testing suite is a tick-based fuzzer located in `src/bin/fuzz.rs`. Unlike traditional integration tests that rely on real time and asynchronous events, this fuzzer controls the progression of time explicitly.

### Architecture

The fuzzer operates in a strict loop:
1. **`sim.step()`**: Advances the simulation by a single discrete "tick".
2. **Invariant Checking**: Immediately after the step, the fuzzer pulls the internal state of every node in the cluster and verifies cluster-wide invariants.

This "stop-the-world" approach ensures that even transient invariant violations—which might be missed in a concurrent environment—are captured the moment they occur.

## Invariants Checked

The checker lives in `src/invariants/`, with one file per property. Each property is tagged with its authoritative source (Ongaro's paper/dissertation or Vanlightly's [TLA+ spec](https://github.com/Vanlightly/raft-tlaplus/blob/main/specifications/standard-raft/Raft.tla)).

| Property                | Paper §  | TLA+ invariant                  | What it checks                                                                       |
|-------------------------|----------|---------------------------------|--------------------------------------------------------------------------------------|
| Election Safety         | §3.6.3   | (implicit)                      | At most one leader per `CommittedLeaderId`                                           |
| Log Matching            | §3.5     | `NoLogDivergence`               | Same index ⇒ same committed-leader id on any two committed logs                      |
| Leader Completeness     | §3.6.3   | `LeaderHasAllAckedValues`       | A later leader must have every entry committed by any earlier leader                 |
| State Machine Safety    | §3.6.3   | (implicit)                      | Same applied log id ⇒ identical state machine data                                   |
| Committed On Quorum     | —        | `CommittedEntriesReachMajority` | A leader's own just-committed index is present on a voter quorum                     |
| Committed Immutable     | derived  | (history-based)                 | Once an index is reported committed, its leader id never changes across ticks        |
| State Ordering          | —        | (implementation sanity)         | `purged ≤ snapshot ≤ applied ≤ committed ≤ last_log` on every node                   |
| Monotonic Term          | §3.3     | `MonotonicTerm`                 | A node's `current_term` never decreases across ticks                                 |
| Monotonic Commit Index  | §3.4     | `MonotonicCommitIndex`          | A node's `committed.index` never decreases across ticks                              |
| Monotonic Applied Index | derived  | —                               | A node's `last_applied.index` never decreases across ticks                           |
| Monotonic Vote          | §3.3     | `MonotonicVote`                 | A node's persisted vote (ordered by `(term, leader_id, committed)`) never regresses  |

All identity comparisons use `CommittedLeaderId` (not just `term`), so the same checks are correct under both openraft modes:
- `leader_id_std`: `CommittedLeaderId == term`, so Election Safety reduces to "one leader per term".
- `leader_id_adv` (the default, used here): `CommittedLeaderId == (term, node_id)`, so two nodes legitimately leading with different `node_id`s in the same term are not flagged.

The checker is **stateful**: `InvariantChecker` retains per-index committed history (for Committed Immutable) and per-node last-seen state (for the Monotonic family) across ticks, so cross-time invariants can be caught. `Leader Append-Only` (paper §3.6.3) is not checked directly; its safety content for committed entries is already covered by Log Matching + Committed Immutable + Leader Completeness.

Unit tests under `src/invariants/tests.rs` exercise each check with synthetic snapshots — run with `cargo test --lib invariants`.

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

### 4. Committed-State Focus
To avoid false positives, log-consistency invariants only compare entries both sides have committed. Uncommitted entries can legitimately diverge during leader transitions and only stabilize once committed. See [Invariants Checked](#invariants-checked) for the full list of properties.

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
  - LogMatching: index=4 differs: n1@T1 vs n5@T2

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
