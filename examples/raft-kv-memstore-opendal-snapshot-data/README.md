# Example Openraft kv-store with snapshot stored in remote storage

With `generic-snapshot-data` feature flag enabled, Openraft allows application to use any data type for snapshot data,
instead of a single-file like data format with `AsyncSeek + AsyncRead + AsyncWrite + Unpin` bounds.

This example shows how to save and retrieve snapshot data from remote storage, allowing users to follow a similar pattern for implementing business logic such as snapshot backups.

This example is similar to the basic raft-kv-memstore example
but focuses on how to store and fetch snapshot data from remote storage.
Other aspects are minimized.

To send a complete snapshot, Refer to implementation of `RaftNetwork::full_snapshot()` in this example.

To receive a complete snapshot, Refer to implementation of `api::snapshot()` in this example.


## Run it

Run it with `cargo test -- --nocaputre`.