//! This fjall-backed storage implements the [`openraft::storage::RaftStateMachine`] trait.
//!
//! [`FjallStateMachine`] keeps the application data and the applied state in a
//! [`fjall::Database`]. Every `apply()` writes its entries and the applied log id in one atomic
//! batch and syncs it before responding, so the state machine is durable. A snapshot is built
//! from a fjall snapshot, a read view of the database, while new entries keep being applied.
#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]
#![allow(clippy::uninlined_format_args)]

mod snapshot_builder;
mod state_machine;

#[cfg(test)]
mod test;

pub use crate::snapshot_builder::FjallSnapshotBuilder;
pub use crate::state_machine::FjallStateMachine;

#[cfg(test)]
openraft::declare_raft_types!(
    /// The type configuration the tests run with.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
);
