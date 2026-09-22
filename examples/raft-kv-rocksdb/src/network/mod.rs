//! Raft RPC over HTTP, with snapshots streamed as `dir-transfer` frames.
//!
//! Log replication, votes, and leader transfer reuse the JSON endpoints of `network-v2-http`.
//! A snapshot is a RocksDB checkpoint directory, so `full_snapshot` sends it as one
//! `POST /snapshot` whose body is a stream of length-prefixed MessagePack messages: first the
//! leader's vote and the snapshot metadata, then every frame `DirFrameProducer` emits. The server
//! writes the frames into a new generation directory of the state machine, then installs it
//! with `Raft::install_full_snapshot` and answers with the `SnapshotResponse`.

mod client;
mod server;
mod snapshot_stream;

pub use client::Network;
pub use client::NetworkFactory;
pub use server::Server;
