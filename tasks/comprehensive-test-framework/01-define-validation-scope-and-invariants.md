# 01 - Define the validation scope, invariants, and bug classes

## Objective

Define the exact safety, liveness, and risk coverage that the automated validation framework must provide.

## Implementation steps

- Write a new design note under the existing documentation directories.
- Prefer `openraft/src/docs/`; use `guide/` instead if the content belongs in the book-style docs.
- List the safety invariants the test framework must assert: leader uniqueness, log matching, state machine safety, vote monotonicity, and membership-change safety.
- Add a second section for liveness expectations with explicit pass conditions such as leader election completes within a bounded simulated time, committed writes eventually replicate after transient failures, and a restarted quorum can make progress again.
- Create a table that maps each bug class to at least one automated test layer, e.g. deterministic simulation, property tests, fault injection, compatibility tests, or soak tests.
- For every invariant in the table, identify the current test files that already cover it and the gaps that still need new tests.

## Deliverables

- one checked-in scope document;
- one bug-class-to-test-layer matrix;
- one gap list that names the missing tests to add.
