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
  - Deliverables:
    - safety invariants: leader uniqueness, log matching, state machine safety, membership-change safety;
    - liveness expectations: election progress, replication progress, recovery after transient faults;
    - risk matrix that maps bug classes to test layers.

- [ ] Build a deterministic cluster simulation harness for core Raft behavior.
  - Cover leader election, replication, commit advancement, membership changes, snapshot installation, learner promotion, leader transfer, restart, and recovery.
  - Make scheduling, timing, message delivery, and storage outcomes reproducible from a seed.

- [ ] Add model-based and state-machine differential tests.
  - Compare OpenRaft behavior against a smaller executable model for selected scenarios.
  - Verify that committed histories and node states satisfy the documented invariants after each step.

- [ ] Expand property-based and fuzz testing around protocol transitions.
  - Generate randomized sequences of writes, membership changes, crashes, restarts, partitions, snapshotting, and compaction.
  - Persist failing seeds and make them replayable in CI and locally.

- [ ] Introduce network fault-injection coverage.
  - Exercise packet loss, duplication, reordering, delay, partition, heal, asymmetric connectivity, and split-brain pressure.
  - Validate both short transient failures and long partition windows.

- [ ] Introduce storage and persistence fault-injection coverage.
  - Exercise torn writes, delayed fsync, partial snapshot availability, log truncation edges, corruption detection, and restart-after-failure paths.
  - Verify crash consistency and recovery expectations for storage implementations.

- [ ] Add concurrency and race-focused testing.
  - Stress overlapping client writes, replication backpressure, snapshot/log races, shutdown during I/O, and concurrent admin operations.
  - Run with tools and schedules that maximize detection of ordering-sensitive bugs.

- [ ] Build compatibility and upgrade test matrices.
  - Verify mixed-version cluster behavior where supported.
  - Cover snapshot compatibility, log compatibility, legacy APIs, runtime variants, and storage implementations.

- [ ] Add end-to-end black-box scenario suites for examples and reference deployments.
  - Run production-like cluster workflows through public APIs only.
  - Validate bootstrap, scaling, failover, restore, and rolling-restart flows.

- [ ] Add long-running soak, stress, and regression-detection jobs.
  - Run extended randomized workloads and fault campaigns.
  - Track flakiness, performance regressions, deadlocks, hangs, and memory/resource growth.

- [ ] Wire the framework into CI with tiered quality gates.
  - Fast gate on every PR: deterministic targeted suites and replay of known regression seeds.
  - Broader gate on main/nightly: randomized campaigns, long-running stress, compatibility matrix, and artifact retention.
  - Release gate: full validation matrix before cutting releases.

- [ ] Improve failure reporting and artifact capture.
  - Store seeds, schedules, logs, node timelines, snapshots, and minimized reproductions for failed runs.
  - Ensure CI output is actionable enough for unattended triage.

- [ ] Add flake management and quarantine policy.
  - Define what blocks merges, what may be quarantined temporarily, and how quarantined coverage is tracked back to green.
  - Require every escaped bug to become a reproducible regression test.

## Suggested rollout order

1. Define invariants and scope.
2. Add deterministic simulation for critical protocol paths.
3. Add randomized/property-based coverage with replayable seeds.
4. Add network/storage fault injection.
5. Add compatibility, upgrade, and end-to-end suites.
6. Add long-running soak jobs and CI gating policy.

## Definition of done

- [ ] Every proposed sub-task above has been filed and linked from the parent task or issue.
- [ ] Each merged sub-task adds or expands automated coverage, not just infrastructure.
- [ ] The repository documents how to run each test layer locally and in CI.
- [ ] New patches are required to pass the appropriate automated gates before merge.
- [ ] Escaped bugs are fed back into the framework as permanent regression coverage.
