//! Items that `openraft-legacy` exports under its own paths.
//!
//! `openraft-legacy` depends on this crate, so a type that both crates must name is defined here,
//! and `openraft-legacy` re-exports it from this module. Not part of the public API: use the paths
//! in `openraft-legacy`.

pub use crate::raft::message::AppendEntriesResponse;
