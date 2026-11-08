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
   The same application also yields the target configuration, the one the request describes.

2. **Second Phase:** Append the target configuration computed in the first phase. This transitions the cluster from
   joint configuration to a uniform configuration and is then committed.

When OpenRaft can prove that every quorum of the current configuration intersects every quorum of the new one, such as
when the change adds or removes one voter, the first phase writes the new uniform configuration directly, and there is
no second phase.

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

The second phase no longer interprets the request again. It appends the target configuration the first phase computed
from it, and it requires the last membership log to still be the joint configuration of its own first phase. A
concurrent membership change therefore cannot make the second phase append a configuration that neither request asked
for: a change whose joint configuration has been replaced fails the precondition, and the cluster ends in the target of
the change that replaced it.

Earlier versions of this solution applied an empty change request (`AddVoterIds({})`) to the last configuration in
the joint config, which also ends in a uniform configuration. But that discards the result of the request: the joint
configuration still holds every member the request removes, so a member the request removes explicitly can reappear as
a learner instead of leaving the cluster.

## Impact

- **Single Change Request:** No behavior changes occur if only one membership change request is in progress.
- **Concurrent Requests:** If multiple requests are processed concurrently, the application must still verify the
  result: the requests that lose the precondition fail, and the cluster ends in the target of the last request to
  append its joint configuration.

## Acknowledgments

Thanks to @tvsfx for providing feedback on this issue and offering a detailed explanation of the solution.
