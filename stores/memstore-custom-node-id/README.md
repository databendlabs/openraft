# openraft-memstore-custom-node-id

A minimal in-memory Raft storage implementation used to verify that the
openraft storage test suite works correctly with a `NodeId` type whose
`Display` output is **not** a plain integer.

## Purpose

The standard `memstore` crate uses `u64` as `NodeId`, so `NodeId::display()` and
`u64::to_string()` produce identical output.  This crate defines a newtype

```rust
struct NodeId(u64);  // Display → "Node[0]", "Node[1]", …
```

and runs the full [`Suite`](../../openraft/src/testing/log/suite.rs) against it.
Any test that accidentally hardcodes the numeric string representation of a
`NodeId` will fail here but pass in `memstore`, making this crate a targeted regression guard for the bug first reported in
[issue #1659: Storage test suite should not depend on NodeId Display format][issue #1659].

[issue #1659]: https://github.com/databendlabs/openraft/issues/1659

## What is stripped out

This crate intentionally omits everything that is not required by the test suite:

- No application request/response types beyond the required `AppReq` stub.
- No blocking/fault-injection infrastructure (`BlockConfig`, etc.).
- No feature flags for alternative `LeaderId` implementations.
- No snapshot builder counters or other testing knobs.

The goal is the smallest storage implementation that exercises the full suite
with a custom `NodeId::Display`.
