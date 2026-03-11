# Comprehensive automated test framework task

This repository change does **not** create an external issue directly.
Instead, it provides a document that explains the work to do and can be copied into an issue tracker if needed.

## Goal

Build a layered automated validation framework that exercises OpenRaft through deterministic, randomized, adversarial, compatibility, and long-running production-like scenarios, so every patch is screened automatically for safety and regression risks before merge.

> **Reality check:** no finite automated test suite can literally guarantee correctness for all possible executions. To get as close as practical to unattended correctness checks, this plan combines exhaustive techniques where possible with broad fault-injection and compatibility coverage.

## Why this task is needed

This work spans multiple crates, CI jobs, and test styles. It should be tracked as one parent task with linked sub-tasks so the coverage gaps, dependencies, and rollout order stay visible.

## Exit criteria

- [ ] All critical Raft safety invariants are documented and mapped to automated checks.
- [ ] Core protocol paths have deterministic simulation coverage for success and failure cases.
- [ ] Randomized and property-based testing continuously explores protocol state-space.
- [ ] Storage, network, runtime, and restart fault injection are covered in CI.
- [ ] Compatibility, upgrade, and snapshot/log format checks run automatically.
- [ ] Long-running stress and soak jobs detect flakiness, races, and resource leaks.
- [ ] CI has tiered gates so fast checks block every PR and deeper suites run on schedule and before release.
- [ ] Failure artifacts make unattended diagnosis possible without requiring manual reproduction.

## Proposed sub-tasks

- [ ] Define the validation scope, invariants, and bug classes that the automated framework must cover.
  - Implementation steps:
    - Write a new design note under repository documentation that lists the safety invariants the test framework must assert: leader uniqueness, log matching, state machine safety, vote monotonicity, and membership-change safety.
    - Add a second section for liveness expectations with explicit pass conditions such as leader election completes within a bounded simulated time, committed writes eventually replicate after transient failures, and a restarted quorum can make progress again.
    - Create a table that maps each bug class to at least one automated test layer, e.g. deterministic simulation, property tests, fault injection, compatibility tests, or soak tests.
    - For every invariant in the table, identify the current test files that already cover it and the gaps that still need new tests.
  - Deliverables:
    - one checked-in scope document;
    - one bug-class-to-test-layer matrix;
    - one gap list that names the missing tests to add.

- [ ] Build a deterministic cluster simulation harness for core Raft behavior.
  - Implementation steps:
    - Add or extend a test harness crate/module that can construct a multi-node OpenRaft cluster using deterministic clocks, seeded randomness, and in-memory transport/storage.
    - Expose operations for ticking timers, delivering or dropping messages, restarting nodes, forcing snapshots, and inspecting committed logs and membership state.
    - Encode a fixed set of scenario tests for leader election, log replication, commit advancement, membership changes, learner promotion, leader transfer, snapshot installation, restart, and recovery.
    - Ensure every scenario can run from a fixed seed and prints the seed in failures so the execution is exactly reproducible.
  - Deliverables:
    - one reusable deterministic harness API;
    - one initial scenario suite that covers the critical protocol paths;
    - one short README section describing how to run a seeded replay locally.

- [ ] Add model-based and state-machine differential tests.
  - Implementation steps:
    - Implement a small reference model for the Raft states and transitions that are practical to compare, such as elections, append acceptance/rejection, commit advancement, and membership transitions.
    - Run the same generated sequence of cluster operations against both the reference model and the real OpenRaft harness.
    - After each step, compare externally visible outcomes: leader identity, accepted log shape, committed index, membership state, and whether a client write is considered committed.
    - Fail fast with a minimized trace that shows the operation sequence, seed, and the first divergent state.
  - Deliverables:
    - one executable reference model;
    - one differential test runner;
    - one failure report format that makes divergences easy to reproduce.

- [ ] Expand property-based and fuzz testing around protocol transitions.
  - Implementation steps:
    - Add generators for operation sequences that mix client writes, membership changes, crashes, restarts, partitions, heals, snapshot creation, snapshot installation, and compaction.
    - Add shrinking rules so failures reduce to the smallest useful reproducer instead of leaving large random traces.
    - Save failing seeds and reduced traces into a checked-in regression corpus or replay fixture directory.
    - Add one deterministic replay test per fixed failure seed so every previously found bug becomes a permanent regression test.
  - Deliverables:
    - one property-based test entry point;
    - one replayable regression corpus;
    - one documented process for promoting failures into fixed regression tests.

- [ ] Introduce network fault-injection coverage.
  - Implementation steps:
    - Extend the transport layer used by tests so it can inject packet loss, duplication, reordering, bounded delay, long delay, one-way partitions, full partitions, and heals.
    - Add named scenarios for short transient failures and long partitions, and define the expected cluster outcome for each scenario.
    - Add assertions that safety invariants still hold during the fault window and that liveness returns after the fault is healed when quorum is restored.
    - Capture a per-node event timeline in failures so message behavior during the injected fault is visible.
  - Deliverables:
    - one fault-injecting test transport;
    - one scenario matrix for supported network faults;
    - one failure log format that includes message and node timelines.

- [ ] Introduce storage and persistence fault-injection coverage.
  - Implementation steps:
    - Add a storage wrapper used in tests that can inject write failures, delayed persistence, truncated logs, missing snapshot chunks, checksum mismatch, and restart after partial persistence.
    - Define which errors should be surfaced to callers, which should trigger retry behavior, and which should stop the node and require operator intervention.
    - Add restart-and-recover tests that verify state after a crash matches the documented crash-consistency contract.
    - Run the same storage fault cases against each supported storage implementation that participates in CI.
  - Deliverables:
    - one fault-injecting storage adapter;
    - one crash-consistency expectation document;
    - one reusable storage fault test suite.

- [ ] Add concurrency and race-focused testing.
  - Implementation steps:
    - Create targeted tests that overlap client writes, replication backpressure, snapshot creation, snapshot installation, shutdown during I/O, membership changes, and administrative operations.
    - Run these tests under deterministic schedules first, then under randomized schedules that vary operation ordering and timing.
    - Add assertions that no duplicate commits, lost commits, stale leadership decisions, or invalid membership states appear under overlap.
    - Mark the most sensitive tests for repeated execution in CI so they run multiple times with different seeds or schedules.
  - Deliverables:
    - one concurrency stress suite;
    - one list of invariants checked under overlap;
    - one CI configuration that repeats race-sensitive tests.

- [ ] Build compatibility and upgrade test matrices.
  - Implementation steps:
    - Enumerate the supported upgrade paths, legacy APIs, runtime variants, snapshot formats, and storage backends that must interoperate.
    - Build compatibility tests that start a cluster on an old version or legacy path, perform writes and snapshots, then restart part or all of the cluster on the newer implementation.
    - Verify that logs, snapshots, membership state, and client-visible committed data remain readable and valid after upgrade.
    - Add explicit pass/fail rules for unsupported version mixes so CI distinguishes unsupported combinations from regressions.
  - Deliverables:
    - one supported compatibility matrix;
    - one mixed-version or mixed-mode scenario suite;
    - one clear list of unsupported combinations and expected failures.

- [ ] Add end-to-end black-box scenario suites for examples and reference deployments.
  - Implementation steps:
    - Pick the example applications and reference deployments that should serve as black-box coverage targets.
    - For each target, write scenarios that use only public APIs and process boundaries, without reaching into internal test helpers.
    - Cover bootstrap, adding nodes, removing nodes, failover, restore from snapshot, and rolling restart while verifying client-visible reads and writes.
    - Store scenario logs and cluster state snapshots on failure so breakage can be diagnosed without re-running locally first.
  - Deliverables:
    - one black-box suite per selected example or reference deployment;
    - one shared scenario runner that exercises public APIs only;
    - one artifact collection rule for failures.

- [ ] Add long-running soak, stress, and regression-detection jobs.
  - Implementation steps:
    - Define a small number of long-running workloads that combine writes, membership changes, snapshots, restarts, and injected faults over hours rather than minutes.
    - Add metrics collection for throughput, latency, memory growth, open file count, task count, and retry/error counts while the workloads run.
    - Set concrete thresholds that make a run fail on deadlock, hang, unbounded memory growth, or major performance regression.
    - Persist historical run summaries so repeated instability can be recognized as a flake or trend rather than a one-off failure.
  - Deliverables:
    - one soak job definition;
    - one metrics and threshold specification;
    - one historical summary artifact per run.

- [ ] Wire the framework into CI with tiered quality gates.
  - Implementation steps:
    - Split the test framework into tiers: PR-fast, main/nightly, and release-candidate.
    - Put deterministic targeted suites and replay of known bad seeds in the PR-fast tier so every pull request gets fast signal.
    - Put randomized campaigns, compatibility suites, and longer stress/soak jobs in scheduled or main-branch workflows with artifact retention enabled.
    - Add one release gate workflow that runs the full validation matrix and blocks release publication on failure.
  - Deliverables:
    - one documented CI tier map;
    - one workflow update per tier;
    - one artifact-retention policy that states what is kept and for how long.

- [ ] Improve failure reporting and artifact capture.
  - Implementation steps:
    - Standardize the data captured on failure: random seed, schedule, operation trace, node logs, message timeline, snapshot metadata, and minimized reproducer if available.
    - Write failures to a predictable directory layout so CI artifacts are easy to inspect and local reruns can reuse the same inputs.
    - Print a short replay command in the test failure output that tells a developer how to rerun the failing scenario locally.
    - Add one smoke test that intentionally fails in a controlled way and asserts the artifact bundle is created.
  - Deliverables:
    - one standard artifact schema;
    - one local replay instruction format;
    - one test that verifies failure artifacts are emitted.

- [ ] Add flake management and quarantine policy.
  - Implementation steps:
    - Define which test tiers block merges immediately, which tests may be quarantined temporarily, and who is responsible for restoring quarantined coverage.
    - Add a quarantine list that requires an owner, root-cause issue, first-failed date, and removal criteria for each flaky test.
    - Add a periodic review step that fails if quarantined tests age past the allowed limit without an owner update.
    - Require every production escape or CI-found bug to add a deterministic reproducer or replay seed before the fix is considered complete.
  - Deliverables:
    - one written quarantine policy;
    - one tracked quarantine list format;
    - one rule that ties escaped bugs to permanent regression coverage.

## Suggested rollout order

1. Define invariants and scope.
2. Add deterministic simulation for critical protocol paths.
3. Add randomized/property-based coverage with replayable seeds.
4. Add network/storage fault injection.
5. Add compatibility, upgrade, and end-to-end suites.
6. Add long-running soak jobs and CI gating policy.

## Definition of done

- [ ] Every proposed sub-task listed in the "Proposed sub-tasks" section has been filed and linked from the parent task or issue.
- [ ] Each merged sub-task adds or expands automated coverage, not just infrastructure.
- [ ] The repository documents how to run each test layer locally and in CI.
- [ ] New patches are required to pass the appropriate automated gates before merge.
- [ ] Escaped bugs are fed back into the framework as permanent regression coverage.
