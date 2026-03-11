# 12 - Improve failure reporting and artifact capture

## Objective

Make failures diagnosable without requiring manual reproduction first.

## Implementation steps

- Standardize the data captured on failure: random seed, schedule, operation trace, node logs, message timeline, snapshot metadata, and minimized reproducer if available.
- Write failures to a predictable directory layout so CI artifacts are easy to inspect and local reruns can reuse the same inputs.
- Print a short replay command in the test failure output that tells a developer how to rerun the failing scenario locally.
- Add one smoke test that intentionally fails in a controlled way and asserts the artifact bundle is created.

## Deliverables

- one standard artifact schema;
- one local replay instruction format;
- one test that verifies failure artifacts are emitted.
