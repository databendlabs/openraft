### Does OpenRaft support calling `change_membership` in parallel?

Yes, OpenRaft does support this scenario, but with some important caveats.
`change_membership` is a two-step process—first transitioning to a joint
configuration and then to a final uniform configuration. When multiple
`change_membership` calls occur concurrently, their steps can interleave,
potentially leaving the cluster in a joint config state. This state is valid in
OpenRaft.

Here's an example of how such interleaving might play out:

1. **Initial State**: `{y}`
2. **Task 1** calls `change_membership(AddVoters(x))`
   → Transitions to joint config `[{y}, {y, x}]`
3. **Task 2** calls `change_membership(RemoveVoters(x))`
   → Transitions from `[{y}, {y, x}]` to `[{y, x}, {y}]`
4. **Task 2 (Step 2)** finalizes config to `{y}` and reports success to its client
5. **Task 1 (Step 2)** proceeds unaware, adding `x` again
   → Transitions from `{y}` to `[{y}, {y, x}]`

At this point, both tasks report success to their respective clients, but the
cluster is left in a **joint configuration** state: `[{y}, {y, x}]`.

This illustrates how concurrent changes can lead to a final configuration that
may not reflect the full intent of either request. However, this does **not**
violate OpenRaft's consistency model, and joint configs are treated as valid
operating states.

If your system ensures only one process issues `change_membership` at a time,
you're safe. If not, always validate the final config after changes to avoid
surprises.
For example, if two requests attempt to change the config to `{a,b,c}` and
`{x,y,z}` respectively, the result may end up being one or the
other—unpredictable from each individual caller's perspective.

This behavior is by design, as it provides several advantages:

- **Simplifies recovery**: If the leader crashes after the first step, a new
  leader can continue operating from the joint state without requiring special
  recovery procedures.

- **Supports dynamic flexibility**: The cluster can adapt to changing conditions
  without being locked into a specific membership transition.

OpenRaft intentionally supports this behavior because:

- When a leader crashes after establishing a joint configuration, the new leader
  can seamlessly resume from this state and process new membership changes as
  needed.

- The system can adapt if nodes in the target configuration become unstable. For
  example, if transitioning from `{a,b,c}` to `{b,c,d}` but node `d` becomes
  unreliable when it finished phase-1 and the config is `[{a,b,c}, {b,c,d}]`,
  the leader can pivot to include a different node (e.g., change membership
  config to `[{a,b,c}, {b,c,x}]` then to `{b,c,x}`) or revert to the original
  configuration `{a,b,c}`.
