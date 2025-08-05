# Joint Consensus in OpenRaft

## Introduction

Joint consensus is a mechanism in the Raft protocol that allows for safe changes to cluster membership.
When membership changes occur (such as adding or removing nodes), the cluster transitions through a joint configuration
phase where both old and new configurations are considered valid. This ensures that the cluster maintains availability
and safety during transitions.

## Membership Change Process

OpenRaft implements membership changes using a two-phase joint consensus approach:

1. **First Phase:** Apply a membership change request (e.g., `AddVoters({x})`) to the current configuration, forming a
   joint configuration that contains both the old and new configurations. This joint configuration is then committed.

2. **Second Phase:** Apply an **empty change request** (`AddVoterIds({})`) to the last configuration in the current
   joint config. This transitions the cluster from joint configuration to a uniform configuration and is then committed.

## Problem in Previous Versions of OpenRaft (prior to 2025-04-03)

In earlier implementations, the second phase of a membership change would reapply the original change operation to the
current membership state. When multiple membership change requests were processed concurrently, this approach could
leave the system in a joint configuration state rather than transitioning to a uniform configuration.
Consider this example:

- Initial configuration: `{a, b, c}`
- Task 1: `AddVoterIds(x)`
  - After first phase: `[{a, b, c}, {a, b, c, x}]`
- Task 2: `RemoveVoters(x)` (runs concurrently)
  - After first phase: `[{a, b, c, x}, {a, b, c}]` (applied to the last configuration `{a, b, c, x}`)
- Task 1 proceeds to second phase, reapplies `AddVoterIds(x)` to the current state
  - Result: `[{a, b, c}, {a, b, c, x}]` (still a joint configuration)

This behavior was problematic because:
1. The system remained in a joint configuration state indefinitely
2. This contradicted the standard Raft expectation that membership changes should eventually result in a uniform configuration
3. It created confusion for users who expected membership changes to be fully applied

## Solution

The second step now applies an **empty change request** (`AddVoterIds({})`) to the last configuration in the current
joint config. This ensures that the system always transitions to a uniform configuration in the second step, regardless
of concurrent membership operations.

## Impact

- **Single Change Request:** No behavior changes occur if only one membership change request is in progress.
- **Concurrent Requests:** If multiple requests are processed concurrently, the application must still verify the result,
  but the new behavior ensures the system always transitions to a uniform state.

## Acknowledgments

Thanks to @tvsfx for providing feedback on this issue and offering a detailed explanation of the solution.
