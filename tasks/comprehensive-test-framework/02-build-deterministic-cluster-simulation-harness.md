# 02 - Build a deterministic cluster simulation harness

## Objective

Provide a reusable, seed-driven harness for deterministic multi-node Raft scenarios.

## Implementation steps

- First inspect the existing test utilities under `tests/`, `openraft/`, and the example crates.
- If an internal harness already exists, extend it instead of creating a parallel one.
- Otherwise add one new test-only harness module instead of creating multiple overlapping harnesses.
- Expose operations for ticking timers, delivering or dropping messages, restarting nodes, forcing snapshots, and inspecting committed logs and membership state.
- Encode a fixed set of scenario tests for leader election, log replication, commit advancement, membership changes, learner promotion, leader transfer, snapshot installation, restart, and recovery.
- Ensure every scenario can run from a fixed seed and prints the seed in failures so the execution is exactly reproducible.

## Deliverables

- one reusable deterministic harness API;
- one initial scenario suite that covers the critical protocol paths;
- one short section in the new harness README, or in `tests/README.md` if the harness lives under `tests/`, describing how to run a seeded replay locally.
