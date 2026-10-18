# OpenRaft Turmoil Simulation Tests

This package provides a deterministic simulation environment for OpenRaft using the [Turmoil](https://github.com/tokio-rs/turmoil) framework. It is designed to detect deep protocol violations and edge cases by simulating complex network conditions like partitions, packet loss, and message delays, and storage that flushes late, reports flushes out of order and loses unflushed writes in a crash, in a controlled, repeatable manner.

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
- Storage faults: see below.

### Storage faults

Each node's log store (`src/store.rs`) keeps two copies of what it holds: what a reader sees, and what is on disk. `StorageModel` (`src/storage_model.rs`) sets how far they drift apart. Each switch departs from a synchronous, lossless store only as far as openraft's storage contract allows: the `RaftLogStorage` and `IOFlushed` docs, and the `log-rocks`, `log-wal` and `sm-rocks` example stores.

| Switch | Models | Contract it relies on |
|--------|--------|-----------------------|
| `async_flush`, `flush_delay_ms` | `append` returns once its entries are readable; a flush task makes writes durable a latency later, in submission order, and only then reports the appends flushed | the callback may fire after `append` returns; write IO is serialized, so a crash keeps a prefix of it |
| `flush_stall_chance`, `flush_stall_ms` | the long tail of `fsync` latency | — |
| `callback_jitter_ms` | each flushed append is reported after its own delay, so reports reach openraft out of order | `IOFlushed` keeps a watermark and ignores an older completion |
| `power_loss_chance` | a crash drops every unflushed write: a power loss, or any crash of a store that buffers writes in the process, as `log-wal` does; otherwise the page cache still reaches the disk | `save_vote` is durable before it returns, so it waits for every write before it; `truncate_after`, `purge` and `save_committed` need no flush |
| `apply_delay_ms` | a slow `apply` | the state machine is durable when `apply` returns, as in `sm-rocks` |

The fuzzer draws these switches from each seed, on an RNG stream of its own, so every older field of a seed keeps its value; `--storage durable` turns them all off and replays any seed exactly as before they existed (checked on `FUZZ_DEBUG_OPS` traces), and `--storage lossy` makes every seed flush asynchronously and every crash a power loss. With async flush the fuzzer also crashes an active voter every 1,000–5,000 ticks (`storage_crash_interval`), the leader half of the time; half of these are quick restarts, back within `election_timeout_min`, as under a process supervisor.

A flush, its late report included, stays within a quarter of `heartbeat_interval`, stalls aside. A follower acks an append once it has flushed it, and openraft gives the whole round trip 3/4 of `heartbeat_interval`: with flushes near that deadline every append times out and replication stops for good, which says more about the configuration than about openraft.

## Invariants Checked

The checker lives in `src/invariants/`, with one file per property. Each property is tagged with its authoritative source (Ongaro's paper/dissertation or Vanlightly's [TLA+ spec](https://github.com/Vanlightly/raft-tlaplus/blob/main/specifications/standard-raft/Raft.tla)).

| Property                | Paper §  | TLA+ invariant                  | What it checks                                                                       |
|-------------------------|----------|---------------------------------|--------------------------------------------------------------------------------------|
| Election Safety         | §3.6.3   | (implicit)                      | At most one leader per `CommittedLeaderId`                                           |
| Log Matching            | §3.5     | `NoLogDivergence`               | Same index ⇒ same committed-leader id on any two committed logs                      |
| Leader Completeness     | §3.6.3   | `LeaderHasAllAckedValues`       | A leader must have every entry committed before its term (the term it was committed in, not the one that wrote it) |
| State Machine Safety    | §3.6.3   | (implicit)                      | Same applied log id ⇒ identical state machine data                                   |
| Committed On Quorum     | —        | `CommittedEntriesReachMajority` | An index a leader commits, its own term's or an earlier one's, is on the disks (flushed) of a voter quorum |
| Committed Immutable     | derived  | (history-based)                 | Once an index is reported committed, its leader id never changes across ticks        |
| State Ordering          | —        | (implementation sanity)         | `purged ≤ snapshot ≤ applied ≤ committed ≤ last_log` on every node                   |
| Monotonic Term          | §3.3     | `MonotonicTerm`                 | A node's `current_term` never decreases within one process                           |
| Monotonic Commit Index  | §3.4     | `MonotonicCommitIndex`          | A node's `committed.index` never decreases within one process                        |
| Monotonic Applied Index | derived  | —                               | A node's `last_applied.index` never decreases, restarts included                     |
| Monotonic Vote          | §3.3     | `MonotonicVote`                 | A node's flushed vote (ordered by `(term, leader_id, committed)`) never regresses, restarts included |
| Monotonic SM Keys       | derived  | —                               | Per node and key, the writing log id never decreases and keys never vanish          |
| Log Id Uniqueness       | §3.5     | (implied by `NoLogDivergence`)  | No two nodes hold different entries under one log id: on every append, and over every log after the run |

All identity comparisons use `CommittedLeaderId` (not just `term`), so the same checks are correct under both openraft modes:
- `leader_id_std`: `CommittedLeaderId == term`, so Election Safety reduces to "one leader per term".
- `leader_id_adv` (the default, used here): `CommittedLeaderId == (term, node_id)`, so two nodes legitimately leading with different `node_id`s in the same term are not flagged.

A restart reloads term and committed from disk, which may hold less than the process had: a power loss drops a vote whose `save_vote` had not returned, and an unflushed `save_committed`. So those two are checked within one process; the flushed vote, which storage made durable before openraft reported it, and the applied state, durable when `apply` returns, are checked across restarts too.

Log Id Uniqueness compares payloads, which Log Matching cannot: it compares log ids only, and one log id naming two entries is exactly what openraft [#2167](https://github.com/databendlabs/openraft/issues/2167) produces. An entry that no node holds any more, such as one lost with the power before it left the leader, may be reissued.

The checker is **stateful**: `InvariantChecker` retains per-index committed history (for Committed Immutable) and per-node last-seen state (for the Monotonic family) across ticks, so cross-time invariants can be caught. `Leader Append-Only` (paper §3.6.3) is not checked directly; its safety content for committed entries is already covered by Log Matching + Committed Immutable + Leader Completeness.

Host panics, all-clients-exited, and simulation-duration overrun also fail the run: `sim.step()` errors are converted into violations, and each iteration runs under `catch_unwind`.

Unit tests under `src/invariants/tests.rs`, `src/oracle.rs`, and `src/liveness.rs` exercise each check with synthetic snapshots, and `src/store.rs` the storage model — run with `cargo test --lib`.

## State Access via RaftMetrics

The fuzzer reads node state through `Raft::metrics()`, the existing watch channel that `RaftCore` updates on every state change. Two fields were added to `RaftMetrics` to support invariant checking:

* **`committed: Option<LogId>`** — the last log ID this node knows to be committed.
* **`log_id_list: LogIdList`** (behind the `metrics-logids` feature flag) — per-leader log ID tracking, enabling per-index log entry lookup for log consistency verification.

These are synchronous, non-blocking reads (just cloning the latest watched value), making them safe for high-frequency use in the simulation loop.

## Determinism

Bit-for-bit reproducibility requires closing every entropy source; each one below was found by diffing normalized traces of identical runs:

1. **Election timeout RNG**: the TypeConfig wraps its runtime in `openraft_rt::deterministic_rng::DeterministicRng`, which replaces `thread_rng()` with task-local, seed-derived `SmallRng` instances. Each host is seeded from the iteration seed.
2. **tokio watch wake order**: `tokio::sync::watch` distributes waiters over 8 internal `Notify` instances chosen by tokio's OS-entropy-seeded RNG, so two tasks waiting on the same channel (e.g. two replication streams on `io_submitted_rx`) wake in random relative order. Turmoil 0.7 seeds every host runtime from the simulation seed via tokio's `Builder::rng_seed` (a `tokio_unstable` API — hence the mandatory cfg).
3. **Future selection**: OpenRaft uses `AsyncRuntime::select2`, `select3`, and `select4` to poll futures in argument order. Biased polling gives ready futures deterministic priority.
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

### Storage faults: what they find

Measured on 2026-10-18 against `main` at dee1d078, and against the same tree with the fixes for #2166–#2170 applied: seeds 1–2,000 at `--max-steps 10000`, seeds 1–400 at `--max-steps 100000`. Each failing seed was rerun with a single fix applied to attribute it. At 100,000 steps the purge assertion below fails one seed in 20 and would hide the rest, so those columns come from a build that keeps the larger purge point instead of asserting.

| Bug | seeded, 10k | lossy, 10k | seeded, 100k | lossy, 100k | Caught by |
|-----|-------------|------------|--------------|-------------|-----------|
| [#2167](https://github.com/databendlabs/openraft/issues/2167) restored leader reissues a log id | 6 / 2,000, first at seed 812 | 15 / 2,000, first at seed 19 | 4 / 400, first at seed 25 | 30 / 400, first at seed 13 | Log Id Uniqueness; a follower that keeps entries the restored leader lost, so logs never converge; #2170's panic |
| [#2170](https://github.com/databendlabs/openraft/issues/2170) snapshot ending inside a follower's log | — | 1 of the 15 above | — | 2 of the 30 above | `IOProgress::advance` panics; every case passes with the #2167 fix alone |
| Purge point moves back (new) | — | 1 (seed 630) | 20 / 400 | 22 / 400 | `LogHandler::update_purge_upto` debug assertion |
| Startup purges past the snapshot (new) | — | — | 1 / 400 | 9 / 400, first at seed 60 | `RaftState` validation in `Engine::startup` |
| [#2118](https://github.com/databendlabs/openraft/issues/2118) unflushed log IO marked flushed | — | — | — | — | not found, see below |
| [#2116](https://github.com/databendlabs/openraft/issues/2116) apply after a snapshot purge | — | — | — | — | not found |

With the five fixes, #2167 and #2170 disappear; with the fixes for the two new bugs as well, every seed passes in every mode, 6,000 at 10k and 1,200 at 100k, but one. With `--storage durable` the fuzzer behaves as before these switches existed, and finds the purge assertion (seed 514 at 10k, 20 / 400 at 100k) and, all fixes applied, that one seed: a leader whose transfer of leadership to a non-member never ends stops sending heartbeats, and keeps leading after the others removed it (seed 82 at 100k).

- **#2167** needs leader restore (half the seeds), a power loss on a leader while entries it already replicated are unflushed, and a restart within the election timeout. `--storage lossy` makes every crash a power loss, hence the higher rate.
- **#2118** is not reachable with a store that keeps the contract: `save_vote` returns only once every write before it is durable, so a new leader's tail is flushed before its `UpdateIOProgress` runs. 2,000 lossy seeds found nothing, on top of the other fixes, with the #2118 fix reverted, with a `save_vote` that overtakes pending appends (breaking the contract), or with both. The window needs a new leader that still has appends in flight and commits them before its own flush finishes, a round trip faster than that flush. Committed On Quorum checks such commits.
- **#2116**: about half the seeds apply in batches delayed by up to 5–100 ms, with snapshots every 100–250 entries and purges up to the snapshot.
- **The purge point moves back** whenever `trigger().purge_log(i)` on a leader is deferred because replication still reads log `i`, and a snapshot then completes: its policy purge, `max_in_snapshot_log_to_keep` entries short of it, lands below `i`. Debug builds panic; release builds move the purge point back.
- **Startup purges past the snapshot** after a node applied entries its own log had not flushed (a leader commits on its followers' flushes) and then lost them in a power loss. At restart `get_initial_state` finds `last_log_id < last_applied` and purges up to `last_applied`, beyond the last snapshot. A store pair like `log-rocks` with `sm-rocks` can reach it.

### Depth and cost

One seed takes about 0.5 s at `--max-steps 10000` and 12–14 s at `--max-steps 100000` (one process on a 32-core x86 Linux machine, release build), with no measurable difference between storage modes. Per CPU-hour, short lossy runs find #2167 most often (about 55 times), while long runs are needed for the rest: the startup bug did not show up in 14,000 short runs with storage faults, the purge assertion fails one long seed in 20 but one short seed in 2,000, and membership changes come every 10,000–25,000 ticks, so a 10k run sees at most one. CI runs `--iterations 5 --max-steps 10000` per pull request, a few seconds. A nightly job of about an hour on four processes, two at `--max-steps 10000` and two at `--max-steps 100000`, one of each with `--storage lossy`, covers about 7,000 short and 400 long seeds; a base seed taken from the date (`--seed $(date +%Y%m%d)00000`) moves it to new seeds every night, and `--crash-file` keeps what to replay.

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
| `--storage`    | `seeded`       | Storage faults: drawn from each seed (`seeded`), none (`durable`, as before they existed), or always lossy (`lossy`) |

Environment variables:

- `RUST_LOG` — tracing filter for openraft internals (default `openraft=error,warn`, written to stderr). `RUST_LOG=openraft=debug` gives full traces for determinism diffing.
- `FUZZ_DEBUG_OPS=1` — print one line per client op outcome (`DBG ack/wfail/read/rfail`) for trace comparison.
