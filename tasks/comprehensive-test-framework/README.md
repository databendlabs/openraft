# Comprehensive automated test framework task

This repository change does **not** create an external issue directly.
Instead, it provides a directory of task documents that can be copied into an issue tracker as a parent task plus concrete sub-tasks.

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

## Implementation conventions

- Use repository-relative paths in documentation and implementation notes.
- Unless a sub-task defines a better reason to diverge, write failure artifacts under `artifacts/test-framework/{suite-name}/{run-or-scenario}/{seed-or-run-id}/`.
- Example artifact path: `artifacts/test-framework/deterministic-sim/leader-election/seed-12345/`.
- Use stable filenames inside each artifact directory for logs, metrics, thresholds, traces, snapshots, and replay instructions so CI and local tooling can consume them consistently.

## Sub-task files

- [ ] [01 - Define the validation scope, invariants, and bug classes](./01-define-validation-scope-and-invariants.md)
- [ ] [02 - Build a deterministic cluster simulation harness](./02-build-deterministic-cluster-simulation-harness.md)
- [ ] [03 - Add model-based and differential tests](./03-add-model-based-and-differential-tests.md)
- [ ] [04 - Expand property-based and fuzz testing](./04-expand-property-based-and-fuzz-testing.md)
- [ ] [05 - Introduce network fault-injection coverage](./05-introduce-network-fault-injection-coverage.md)
- [ ] [06 - Introduce storage and persistence fault-injection coverage](./06-introduce-storage-and-persistence-fault-injection.md)
- [ ] [07 - Add concurrency and race-focused testing](./07-add-concurrency-and-race-focused-testing.md)
- [ ] [08 - Build compatibility and upgrade test matrices](./08-build-compatibility-and-upgrade-test-matrices.md)
- [ ] [09 - Add end-to-end black-box scenario suites](./09-add-end-to-end-black-box-scenarios.md)
- [ ] [10 - Add long-running soak, stress, and regression-detection jobs](./10-add-long-running-soak-and-stress-jobs.md)
- [ ] [11 - Wire the framework into CI with tiered quality gates](./11-wire-the-framework-into-ci-tiered-gates.md)
- [ ] [12 - Improve failure reporting and artifact capture](./12-improve-failure-reporting-and-artifact-capture.md)
- [ ] [13 - Add flake management and quarantine policy](./13-add-flake-management-and-quarantine-policy.md)

## Suggested rollout order

1. Define invariants and scope.
2. Add deterministic simulation for critical protocol paths.
3. Add randomized/property-based coverage with replayable seeds.
4. Add network/storage fault injection.
5. Add compatibility, upgrade, and end-to-end suites.
6. Add long-running soak jobs and CI gating policy.

## Definition of done

- [ ] Every sub-task file in this directory has been filed and linked from the parent task or issue.
- [ ] Each merged sub-task adds or expands automated coverage, not just infrastructure.
- [ ] The repository documents how to run each test layer locally and in CI.
- [ ] New patches are required to pass the appropriate automated gates before merge.
- [ ] Escaped bugs are fed back into the framework as permanent regression coverage.
