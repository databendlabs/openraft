# Example Openraft kv-store using `RaftNetworkV2`

With `RaftNetworkV2`, Openraft allows application to use any data type for snapshot data,
instead of a single-file like data format with `AsyncSeek + AsyncRead + AsyncWrite + Unpin` bounds.

This example is similar to the basic raft-kv-memstore example
but focuses on how to handle snapshot with `RaftNetworkV2::full_snapshot()`.
Other aspects are minimized.

To send a complete snapshot, Refer to implementation of `RaftNetworkV2::full_snapshot()` in this example.

To receive a complete snapshot, Refer to implementation of `api::snapshot()` in this example.


## Run it

Run it with `cargo test -- --nocapture`.