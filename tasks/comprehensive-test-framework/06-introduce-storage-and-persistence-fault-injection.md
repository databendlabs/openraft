# 06 - Introduce storage and persistence fault-injection coverage

## Objective

Verify crash consistency, corruption handling, and recovery behavior across storage implementations.

## Implementation steps

- Add a storage wrapper used in tests that can inject write failures, delayed persistence, truncated logs, missing snapshot chunks, checksum mismatch, and restart after partial persistence.
- Define in the crash-consistency expectation document which errors should be surfaced to callers, which should trigger retry behavior, and which should stop the node and require operator intervention.
- Add restart-and-recover tests that verify state after a crash matches the documented crash-consistency contract.
- Run the same storage fault cases against each supported storage implementation that participates in CI.

## Deliverables

- one fault-injecting storage adapter;
- one crash-consistency expectation document;
- one reusable storage fault test suite.
