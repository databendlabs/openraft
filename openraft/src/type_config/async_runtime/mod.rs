//! `async` runtime interface.
//!
//! `async` runtime is an abstraction over different asynchronous runtimes, such as `tokio`,
//! `async-std`, etc.
//!
//! This module re-exports types from the `openraft-async-runtime` crate.

// Re-export all public items from openraft-async-runtime
pub use openraft_rt::AsyncRuntime;
pub use openraft_rt::Instant;
pub use openraft_rt::Mpsc;
pub use openraft_rt::MpscReceiver;
pub use openraft_rt::MpscSender;
pub use openraft_rt::MpscWeakSender;
pub use openraft_rt::Mutex;
pub use openraft_rt::Oneshot;
pub use openraft_rt::OneshotSender;
pub use openraft_rt::RecvError;
pub use openraft_rt::SendError;
pub use openraft_rt::TryRecvError;
pub use openraft_rt::Watch;
pub use openraft_rt::WatchReceiver;
pub use openraft_rt::WatchSender;
pub use openraft_rt::instant;
pub use openraft_rt::mpsc;
pub use openraft_rt::mutex;
pub use openraft_rt::oneshot;
pub use openraft_rt::watch;
#[cfg(feature = "tokio-rt")]
pub use openraft_rt_tokio::TokioInstant;
