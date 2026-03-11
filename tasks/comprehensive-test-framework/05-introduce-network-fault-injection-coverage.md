# 05 - Introduce network fault-injection coverage

## Objective

Verify safety and recovery behavior under realistic transport failures.

## Implementation steps

- Extend the transport layer used by tests so it can inject packet loss, duplication, reordering, bounded delay, long delay, one-way partitions, full partitions, and heals.
- Add named scenarios for short transient failures and long partitions, and define the expected cluster outcome for each scenario.
- Add assertions that safety invariants still hold during the fault window and that liveness returns after the fault is healed when quorum is restored.
- Capture a per-node event timeline in failures so message behavior during the injected fault is visible.

## Deliverables

- one fault-injecting test transport;
- one scenario matrix for supported network faults;
- one failure log format that includes message and node timelines.
