# 03 - Add model-based and differential tests

## Objective

Compare OpenRaft behavior with a smaller executable model for selected scenarios.

## Implementation steps

- Implement a small reference model for the Raft states and transitions that are practical to compare, such as elections, append acceptance or rejection, commit advancement, and membership transitions.
- Run the same generated sequence of cluster operations against both the reference model and the real OpenRaft harness.
- After each step, compare externally visible outcomes: leader identity, accepted log shape, committed index, membership state, and whether a client write is considered committed.
- Fail fast with a minimized trace that shows the operation sequence, seed, and the first divergent state.

## Deliverables

- one executable reference model;
- one differential test runner;
- one failure report format that makes divergences easy to reproduce.
