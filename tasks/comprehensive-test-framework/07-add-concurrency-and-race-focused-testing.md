# 07 - Add concurrency and race-focused testing

## Objective

Increase coverage for ordering-sensitive failures that appear only under overlap or contention.

## Implementation steps

- Create targeted tests that overlap client writes, replication backpressure, snapshot creation, snapshot installation, shutdown during I/O, membership changes, and administrative operations.
- Run these tests under deterministic schedules first, then under randomized schedules that vary operation ordering and timing.
- Add assertions that no duplicate commits, lost commits, stale leadership decisions, or invalid membership states appear under overlap.
- Mark the most sensitive tests for repeated execution in CI so they run multiple times with different seeds or schedules.

## Deliverables

- one concurrency stress suite;
- one list of invariants checked under overlap;
- one CI configuration that repeats race-sensitive tests.
