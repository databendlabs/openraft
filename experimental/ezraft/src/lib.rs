//! EzRaft - A beginner-friendly Raft framework built on openraft
//!
//! EzRaft runs your application on several machines at once and keeps their state identical, so
//! the service survives losing some of them. You write two things - everything that makes the
//! machines agree is handled internally:
//!
//! - The application via [`EzApp`]: request/response types, state, and business logic
//! - Storage persistence via [`EzStorage`]
//!
//! # What a cluster does
//!
//! A *cluster* is a set of *nodes*, each one a process running your application. Every node
//! keeps the same list of requests - the *log* - in the same order. A request is *committed*
//! once a majority of the nodes have stored it, which is the point at which it can no longer be
//! lost. Each node then *applies* committed requests to its own copy of your state, in log
//! order, so every node arrives at the same state. [`EzApp::apply`] is that step.
//!
//! Because every node holds the whole state, any node can answer a read from memory
//! ([`EzRaft::read`]) without talking to anyone. Writes take the longer path: they go through
//! one node elected *leader*, and [`EzRaft::write`] finds it for you.
//!
//! A *snapshot* is the state serialized. It lets a node that has fallen too far behind be
//! caught up in one transfer instead of replaying the log, and it lets old log entries be
//! deleted. EzRaft builds and installs snapshots on its own.
//!
//! What matters for fault tolerance is that a majority survives: three nodes tolerate one
//! failure, five tolerate two.
//!
//! # Quick start
//!
//! A complete key-value node. This is `examples/kvstore.rs` without its command line - run that
//! with `cargo run --example kvstore` to get a cluster going.
//!
//! ```no_run
//! use std::collections::BTreeMap;
//! use std::io;
//!
//! use async_trait::async_trait;
//! use ezraft::EzApp;
//! use ezraft::EzConfig;
//! use ezraft::EzRaft;
//! use ezraft::FileStorage;
//! use serde::Deserialize;
//! use serde::Serialize;
//!
//! // 1. What a client asks the cluster to do. It travels the network and goes into
//! //    the log, hence serde; openraft prints it in its logs, hence Display.
//! #[derive(Serialize, Deserialize, Debug, Clone, derive_more::Display)]
//! enum Request {
//!     #[display("Set({key})")]
//!     Set { key: String, value: String },
//! }
//!
//! // 2. What `apply` hands back to whoever issued the write.
//! #[derive(Serialize, Deserialize, Debug, Clone)]
//! struct Response {
//!     value: Option<String>,
//! }
//!
//! // 3. The application *is* the replicated state. A snapshot is this struct
//! //    serialized, so serde is all it takes.
//! #[derive(Default, Serialize, Deserialize)]
//! struct KvApp {
//!     data: BTreeMap<String, String>,
//! }
//!
//! #[async_trait]
//! impl EzApp for KvApp {
//!     type Request = Request;
//!     type Response = Response;
//!
//!     // Called once per committed entry, in log order, on every node.
//!     async fn apply(&mut self, req: Request) -> Response {
//!         match req {
//!             Request::Set { key, value } => Response {
//!                 value: self.data.insert(key, value),
//!             },
//!         }
//!     }
//!
//!     // Optional: answers `GET /api/read?key=...` from local state.
//!     fn read(&self, key: &str) -> Option<serde_json::Value> {
//!         self.data.get(key).map(|v| serde_json::Value::String(v.clone()))
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> io::Result<()> {
//!     // 4. Where this node's state goes. One directory per node. `FileStorage`
//!     //    is the bundled `EzStorage`; read its caveats before keeping it.
//!     let storage = FileStorage::new("./data/node1").await?;
//!
//!     // 5. The first node of a cluster creates it; every other node joins
//!     //    through any node already in it:
//!     //      EzRaft::join("127.0.0.1:8081", "127.0.0.1:8080", app, storage, config)
//!     let raft = EzRaft::create("127.0.0.1:8080", KvApp::default(), storage, EzConfig::default()).await?;
//!
//!     raft.write(Request::Set {
//!         key: "hello".to_string(),
//!         value: "world".to_string(),
//!     })
//!     .await?;
//!     println!("{:?}", raft.read(|app| app.data.get("hello").cloned()).await);
//!
//!     // 6. Serve the Raft RPCs peers need, plus the app API. This blocks;
//!     //    `tokio::spawn` it when the caller has other work to do.
//!     raft.serve().await
//! }
//! ```
//!
//! # Errors
//!
//! Every fallible method returns [`std::io::Error`], including the ones that fail for reasons
//! that have nothing to do with I/O. This is deliberate: a caller cannot usefully branch on the
//! difference. A write that finds no leader, one that cannot reach the leader, and one issued to
//! a stopped node all mean the same thing to an application -- try again later -- because
//! [`EzRaft::write`] already forwards to the leader on its own.
//!
//! [`EzStorage`] is where a user's own errors originate, and those are I/O errors already, so a
//! second error type would buy nothing and cost a concept. Code that does need to tell the cases
//! apart can reach the underlying openraft node and its typed errors through
//! [`EzRaft::inner`](raft::EzRaft::inner).

pub mod app;
pub mod config;
pub mod entry;
pub mod meta;
pub mod network;
pub mod raft;
pub mod server;
pub mod snapshot;
pub mod storage;
pub mod type_config;

// Re-export public API
pub use app::EzApp;
pub use config::EzConfig;
pub use entry::EzEntry;
pub use entry::EzLogId;
pub use meta::EzMeta;
pub use openraft::RaftTypeConfig;
pub use raft::EzRaft;
pub use snapshot::EzSnapshot;
pub use snapshot::EzSnapshotMeta;
pub use storage::EzStorage;
pub use storage::FileStorage;
pub use storage::Loaded;
pub use storage::Persist;
pub use type_config::EzVote;
pub use type_config::OpenRaftTypes;
