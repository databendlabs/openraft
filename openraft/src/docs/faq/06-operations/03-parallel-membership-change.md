### Does OpenRaft support calling `change_membership` in parallel?

Yes, OpenRaft does support this scenario, but with some important caveats.
A `change_membership` call that needs a joint configuration is a two-step
process—first transitioning to a joint configuration and then to a final
uniform configuration. When multiple `change_membership` calls occur
concurrently, their steps can interleave. The second step of a call requires
the last membership log to still be the joint config of its first step, so a
call whose joint config another call has replaced fails instead of finishing.
A call that stops after its first step, for example because the leader
crashes, leaves the cluster in a joint config state. This state is valid in
OpenRaft.

Here's an example of how such interleaving might play out, where `d` and `e`
are learners:

1. **Initial State**: `{a,b,c}`
2. **Task 1** calls `change_membership(ReplaceAllVoters({a,b,d}))`
   → Transitions to joint config `[{a,b,c}, {a,b,d}]`
3. **Task 2** calls `change_membership(ReplaceAllVoters({a,b,e}))`
   → Transitions from `[{a,b,c}, {a,b,d}]` to `[{a,b,c}, {a,b,d}, {a,b,e}]`
4. **Task 1 (Step 2)** fails with `PreconditionFailed`, because the last
   membership log is now Task 2's joint config
5. **Task 2 (Step 2)** finalizes config to `{a,b,e}` and reports success to its
   client

Task 1's error does not mean that nothing was written: its joint config was
committed, and Task 2 started from it. The cluster ends in Task 2's target
`{a,b,e}` and never reaches Task 1's target `{a,b,d}`.

This illustrates how concurrent changes can lead to a final configuration that
may not reflect the intent of every request. However, this does **not**
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
  config to `[{a,b,c}, {b,c,d}, {b,c,x}]` then to `{b,c,x}`) or revert to the original
  configuration `{a,b,c}`.
