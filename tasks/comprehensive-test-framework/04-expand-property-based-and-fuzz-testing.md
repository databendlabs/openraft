# 04 - Expand property-based and fuzz testing

## Objective

Continuously explore protocol state transitions with randomized but replayable inputs.

## Implementation steps

- Add generators for operation sequences that mix client writes, membership changes, crashes, restarts, partitions, heals, snapshot creation, snapshot installation, and compaction.
- Use the property-testing framework that already exists in the repository if one is present; otherwise choose one framework explicitly in the implementation note before coding.
- Add shrinking rules so failures reduce to the smallest useful reproducer instead of leaving large random traces.
- Save failing seeds and reduced traces into a checked-in regression corpus or replay fixture directory.
- Add one deterministic replay test per fixed failure seed so every previously found bug becomes a permanent regression test.

## Deliverables

- one property-based test entry point;
- one replayable regression corpus;
- one documented process for promoting failures into fixed regression tests.
