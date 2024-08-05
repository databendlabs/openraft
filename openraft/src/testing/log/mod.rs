//! Suite for testing implementations of [`RaftLogStorage`] and [`RaftStateMachine`].
//!
//! [`RaftLogStorage`]: crate::storage::RaftLogStorage
//! [`RaftStateMachine`]: crate::storage::RaftStateMachine

mod store_builder;
mod suite;

pub use store_builder::StoreBuilder;
pub use suite::Suite;
