//! Transfer a flat directory of immutable files as an ordered stream of frames.
//!
//! The protocol is transport-agnostic: this crate defines the frame types and the sender and
//! receiver state machines; the application supplies the carrier — anything that moves opaque
//! frames reliably and in order (TCP, gRPC streaming, an HTTP body, a message queue).
//!
//! A valid stream is:
//!
//! ```text
//! Manifest, { FileStart, Chunk*, FileEnd }, End
//! ```
//!
//! with one `FileStart`/`Chunk`/`FileEnd` group per manifest entry, in manifest order.
//!
//! # Transport contract
//!
//! A transport implements [`FrameSink`] on the sending node and [`FrameSource`] on the receiving
//! node, choosing its own frame encoding; [`send_dir`] and [`recv_dir`] drive one complete
//! session over them. A conforming transport must:
//!
//! 1. Deliver the frames of one session to exactly one receiver, in order, without loss or
//!    duplication.
//! 2. Propagate receiver errors back to the sender; on any error both sides drop the session.
//! 3. Report success only after the receiver's `finish()` succeeds.
//!
//! Retry, authentication, compression, and rate limiting are transport concerns, outside this
//! protocol. A failed or cancelled session is discarded; a new attempt starts from scratch.

mod frame;
mod receiver;
mod sender;
mod transport;

pub use frame::DirFrame;
pub use frame::DirManifest;
pub use frame::FORMAT_VERSION;
pub use frame::FileMeta;
pub use frame::MAX_CHUNK_SIZE;
pub use frame::MAX_NAME_LEN;
pub use receiver::DirReceiver;
pub use sender::DirSender;
pub use transport::FrameSink;
pub use transport::FrameSource;
pub use transport::recv_dir;
pub use transport::send_dir;
