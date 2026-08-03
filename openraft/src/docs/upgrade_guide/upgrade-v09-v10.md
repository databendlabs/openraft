# Guide for upgrading from [v0.9](https://github.com/databendlabs/openraft/tree/release-0.9) to v0.10:

> This guide is under construction; it currently covers the `snapshot_id` changes.

## `snapshot_id` moved out of snapshot metadata

A snapshot is identified by the position it covers: two snapshots at the same
`last_log_id` represent the same state, even when they differ in bytes. The
0.9 `snapshot_id` identified a snapshot *transfer*, not the snapshot, so it no
longer belongs in the metadata. It now lives only in the chunked v1 protocol,
which is the only code that needs it: the receiver compares it against the
in-flight stream to tell a new transfer from a continuation.

API changes:

- [`SnapshotMeta`][] loses its `snapshot_id` field. State machines no longer
  invent an id when building a snapshot:

  ```ignore
  let meta = SnapshotMeta {
      last_log_id,
      last_membership,
  };
  ```

- [`SnapshotSignature`][] loses its `snapshot_id` field.

- A [`RaftNetworkV2`][] (full-snapshot) implementation needs no change.

- A chunked v1 implementation uses `openraft-legacy`, where
  `InstallSnapshotRequest` keeps the exact 0.9 five-field wire layout. Its
  `meta` field is `openraft_legacy::network_v1::SnapshotMeta`, the 0.9-shaped
  type that still carries the id; only the import changes:

  ```ignore
  use openraft_legacy::network_v1::{InstallSnapshotRequest, SnapshotMeta};

  let req = InstallSnapshotRequest {
      vote,
      meta: SnapshotMeta { last_log_id, last_membership, snapshot_id },
      offset,
      data,
      done,
  };
  ```

  The v1 sender now generates a fresh id per transfer session, so a
  retransmitted snapshot is never mistaken for a continuation of an aborted
  one.

No data migration is needed. The serialized layouts of [`SnapshotMeta`][] and
[`SnapshotSignature`][] are unchanged — three fields, with an always-empty
`snapshot_id` written and ignored on read — so snapshots stored by 0.9 load in
0.10 and vice versa, under named and positional (`bincode`, `postcard`)
formats alike.

Mixed-version caveat: request and metadata layouts interoperate with 0.9 peers
in both directions, but error bodies do not — `StorageError` was an enum in
0.9 and is a struct in 0.10 — so a peer of a different version should treat a
serialized error response as diagnostic text rather than parse it.

[`SnapshotMeta`]:      `crate::storage::SnapshotMeta`
[`SnapshotSignature`]: `crate::storage::SnapshotSignature`
[`RaftNetworkV2`]:     `crate::network::RaftNetworkV2`
