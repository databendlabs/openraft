//! Transfer a flat directory of immutable files as an ordered stream of frames.
//!
//! [`DirFrameProducer`] reads directory files and yields [`DirFrame`] values. [`DirWriter`]
//! validates those frames and writes them into a target directory. The application supplies the
//! transport, such as TCP, gRPC streaming, an HTTP body, or a message queue.
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
//! 1. Deliver the frames of one session to exactly one [`DirWriter`], in order, without loss or
//!    duplication.
//! 2. Propagate writer errors back to the sender; on any error both sides drop the session.
//! 3. Report success only after [`DirWriter::finish`] succeeds.
//!
//! Retry, authentication, compression, and rate limiting are transport concerns, outside this
//! protocol. A failed or cancelled session is discarded; a new attempt starts from scratch.

mod frame;
mod frame_producer;
mod transport;
mod writer;

pub use frame::DirFrame;
pub use frame::DirManifest;
pub use frame::MAX_CHUNK_SIZE;
pub use frame::MAX_FILE_NAME_BYTES;
pub use frame::ManifestEntry;
pub use frame::PROTOCOL_VERSION;
pub use frame_producer::DirFrameProducer;
pub use transport::FrameSink;
pub use transport::FrameSource;
pub use transport::recv_dir;
pub use transport::send_dir;
pub use writer::DirWriter;
