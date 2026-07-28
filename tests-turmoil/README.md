# OpenRaft Turmoil Simulation Tests

This package provides a deterministic simulation environment for OpenRaft using the [Turmoil](https://github.com/tokio-rs/turmoil) framework. It is designed to detect deep protocol violations and edge cases by simulating complex network conditions like partitions, packet loss, and message delays in a controlled, repeatable manner.

**Build from this directory, with `RUSTFLAGS` unset.** The workspace requires `--cfg tokio_unstable` (set in `.cargo/config.toml`, which cargo only reads when invoked from inside `tests-turmoil/`, and which any `RUSTFLAGS` environment variable overrides — in CI, `actions-rust-lang/setup-rust-toolchain` exports `RUSTFLAGS="-D warnings"` unless given `rustflags: ""`). Either mistake fails with a deliberate `compile_error!` instead of silently producing a non-deterministic binary.

## Deterministic Fuzzer

The core of this suite is a tick-based fuzzer in `src/bin/fuzz.rs`. Each iteration derives every parameter (cluster size, timeouts, fault mix, workload shape) from a single seed, then runs two phases:

1. **Safety phase**: a client workload (tracked writes, linearizable reads), membership churn, trigger ops, and network/process chaos all run concurrently. After every tick the fuzzer snapshots every node and checks all invariants plus the client oracle.
2. **Liveness phase**: all faults are removed and the cluster must *completely heal*:
   - **Converge**: one leader, uniform (non-joint) membership, every member with identical log, applied index, and state machine. A joint config left behind by an interrupted `change_membership` is finalized by the harness, acting as the operator.
   - **Serve**: the healed cluster must ack fresh writes and serve linearizable reads.
   - **Re-converge and durability scan**: after the service check, every member's settled state machine must hold every acked write (or a later write to the same key).

A cluster that cannot heal without faults has a liveness bug (stuck replication, dead-end progress state, election live-lock).

### Client oracle

`src/oracle.rs` tracks every write attempt by unique serial. Acked writes record the returned `LogId`; failed or timed-out writes are *unknown* (they may still commit later) and are never assumed absent. Each observed value embeds `(serial, writing LogId)`, and every piece of evidence — acks (including the previous value each apply returns), linearizable reads, and final-state scans — feeds these checks:

- **Phantom value**: an observed value must map back to a known write attempt.
- **One serial, one log id**: all sightings of a serial must agree on its log id, and one log id must never carry two different serials.
- **Monotonic reads**: per key, a sequential reader must never observe an older `LogId` than it already saw.
- **Read-your-writes**: a linearizable read must observe at least the acked floor captured before the read started.
- **Predecessor chain**: the previous value an ack's apply returns must be the newest known-committed write to the key below the new entry — committed data cannot vanish between two acks even if no read touches the window.
- **Absence floor**: an apply seeing the key absent, or a read observing it absent at its `ReadIndex` barrier, forbids any write to the key from ever resolving below that point.
- **Durability** (final): after final convergence, no acked write — and no write any read observed — may be lost.

### Chaos and operations

- Network chaos: partitions, holds, per-link latency spikes (jitter), per-link loss, global fail-rate, repair.
- Process chaos: node crashes with short or long outages (long outages force snapshot-based catch-up).
- Trigger ops on random nodes: `elect` (with/without Pre-Vote), `snapshot`, `purge_log`, `transfer_leader`, and `compact` (snapshot then purge-to-snapshot, biased toward the leader — on an idle leader this purges the log to its tip).
- Membership churn: grow/shrink between 3 and 7 voters, reusing removed nodes, with demotion (`retain=true`) and full removal; a failed change retries the same plan after a cooldown.
- Workload shape: quiet windows during the run, plus an optional pre-liveness tail-off so traps armed late in the safety phase survive into the liveness phase.

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
| Monotonic SM Keys       | derived  | —                               | Per node and key, the writing log id never decreases and keys never vanish          |

All identity comparisons use `CommittedLeaderId` (not just `term`), so the same checks are correct under both openraft modes:
- `leader_id_std`: `CommittedLeaderId == term`, so Election Safety reduces to "one leader per term".
- `leader_id_adv` (the default, used here): `CommittedLeaderId == (term, node_id)`, so two nodes legitimately leading with different `node_id`s in the same term are not flagged.

The checker is **stateful**: `InvariantChecker` retains per-index committed history (for Committed Immutable) and per-node last-seen state (for the Monotonic family) across ticks, so cross-time invariants can be caught. `Leader Append-Only` (paper §3.6.3) is not checked directly; its safety content for committed entries is already covered by Log Matching + Committed Immutable + Leader Completeness.

Host panics, all-clients-exited, and simulation-duration overrun also fail the run: `sim.step()` errors are converted into violations, and each iteration runs under `catch_unwind`.

Unit tests under `src/invariants/tests.rs`, `src/oracle.rs`, and `src/liveness.rs` exercise each check with synthetic snapshots — run with `cargo test --lib`.

## State Access via RaftMetrics

The fuzzer reads node state through `Raft::metrics()`, the existing watch channel that `RaftCore` updates on every state change. Two fields were added to `RaftMetrics` to support invariant checking:

* **`committed: Option<LogId>`** — the last log ID this node knows to be committed.
* **`log_id_list: LogIdList`** (behind the `metrics-logids` feature flag) — per-leader log ID tracking, enabling per-index log entry lookup for log consistency verification.

These are synchronous, non-blocking reads (just cloning the latest watched value), making them safe for high-frequency use in the simulation loop.

## Determinism

Bit-for-bit reproducibility requires closing every entropy source; each one below was found by diffing normalized traces of identical runs:

1. **Election timeout RNG**: the TypeConfig wraps its runtime in `openraft_rt::deterministic_rng::DeterministicRng`, which replaces `thread_rng()` with task-local, seed-derived `SmallRng` instances. Each host is seeded from the iteration seed.
2. **tokio watch wake order**: `tokio::sync::watch` distributes waiters over 8 internal `Notify` instances chosen by tokio's OS-entropy-seeded RNG, so two tasks waiting on the same channel (e.g. two replication streams on `io_submitted_rx`) wake in random relative order. The [forked turmoil](https://github.com/tokio-rs/turmoil/compare/v0.6.6...drmingdrmer:turmoil:v0.6.6-openraft.1) seeds every host runtime via `Builder::rng_seed` (a `tokio_unstable` API — hence the mandatory cfg).
3. **`futures_util::select!` shuffle**: its RNG is a process-wide thread-local, deterministic per process but leaking state across in-process iterations, which made `--reproduce` diverge from in-sweep failures. The [forked futures-util](https://github.com/rust-lang/futures-rs/compare/0.3.32...drmingdrmer:futures-rs:0.3.32-openraft.1) adds a `reseed()` hook, called at the start of every iteration.
4. **RPC timeouts**: the network layer bounds every RPC by `RPCOption::soft_ttl()`. Turmoil TCP does not retransmit, so a partition can eat an in-flight response while the connection stays open; without the deadline the replication stream wedges forever, past the point where the network heals.
5. **Committed-state focus**: log-consistency invariants only compare entries both sides have committed; uncommitted entries legitimately diverge during leader transitions.
6. **TCP capacity**: `.tcp_capacity(65536)` absorbs connection bursts under chaos ("server socket buffer full" panics otherwise).

Verify determinism with two identical runs:

```bash
FUZZ_DEBUG_OPS=1 ./target/release/fuzz --reproduce 100 --max-steps 15000 > a.log 2>/dev/null
FUZZ_DEBUG_OPS=1 ./target/release/fuzz --reproduce 100 --max-steps 15000 > b.log 2>/dev/null
cmp a.log b.log
```

This holds down to full `RUST_LOG=openraft=debug` traces (identical after masking wall-clock timestamps).

## Detecting and Reproducing Violations

On a violation the fuzzer prints the failing config, per-node state dump, and the exact reproduce command, then exits 1:

```text
=== FAILED at iteration 54 (seed: 3053) ===
...
Violations:
  - Step 12400: CommittedNotOnQuorum { leader: 1, index: 54, voters: [1, 2, 3, 4, 5, 6], matching: [1, 4, 5] }

REPRODUCE WITH:
  cargo run --bin fuzz -- --reproduce 3053 --max-steps 30000
```

Because iterations are fully deterministic and per-iteration RNG state is reset, an in-sweep failure replays identically in a fresh process with `--reproduce <seed>`.

### Validation against known bugs

The harness was validated by re-introducing a fixed bug and confirming detection: reverting the fix for [#1802](https://github.com/databendlabs/openraft/issues/1802) (`VecProgress` losing its sorted-prefix invariant, committing without a real quorum) is caught within ~50 iterations by `CommittedNotOnQuorum`, reproduces exactly, and disappears when the fix is restored. See `ISSUES.md` for bug classes not yet covered.

## Running the Fuzzer

```bash
cd tests-turmoil            # required: picks up .cargo/config.toml
cargo run --release --bin fuzz -- --seed 500 --iterations 20 --max-steps 50000
```

| Argument       | Default        | Description                                                        |
|----------------|----------------|--------------------------------------------------------------------|
| `--seed`, `-s` | random         | Base seed; iteration `k` uses `seed + k`, which derives all params |
| `--reproduce`  | —              | Run exactly one iteration with this seed                           |
| `--iterations` | 100 (0=∞)      | Number of iterations; stops at the first failure                   |
| `--max-steps`  | 100000         | Safety-phase length in ticks; liveness phases run after            |
| `--crash-file` | —              | Where to write crash info                                          |

Environment variables:

- `RUST_LOG` — tracing filter for openraft internals (default `openraft=error,warn`, written to stderr). `RUST_LOG=openraft=debug` gives full traces for determinism diffing.
- `FUZZ_DEBUG_OPS=1` — print one line per client op outcome (`DBG ack/wfail/read/rfail`) for trace comparison.
