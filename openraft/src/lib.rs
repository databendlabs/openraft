#![doc = include_str!("lib_readme.md")]
#![doc = include_str!("docs/docs.md")]
#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![cfg_attr(feature = "bench", feature(test))]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::bool_comparison)]
// TODO: `clippy::result-large-err`: StorageError is 136 bytes, try to reduce the size.
#![allow(clippy::result_large_err)]
#![allow(clippy::type_complexity)]
#![allow(clippy::uninlined_format_args)]
#![deny(unused_qualifications)]
// TODO: Enable this when doc is complete
// #![warn(missing_docs)]

macro_rules! func_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        let n = &name[..name.len() - 3];
        let nn = n.replace("::{{closure}}", "");
        nn
        // nn.split("::").last().unwrap_or_default()
    }};
}

#[cfg(feature = "loosen-follower-log-revert")]
compile_error!(
    "The feature flag `loosen-follower-log-revert` is removed since `0.10.0`. \
     Use `Config::allow_log_reversion` instead."
);

pub extern crate openraft_macros;

mod change_members;
mod config;
mod core;
mod display_ext;
mod node;
mod progress;
mod quorum;
mod raft_types;
mod replication;
mod runtime;
mod summary;
mod try_as_ref;

pub(crate) mod engine;
pub(crate) mod log_id_range;
pub(crate) mod proposer;
pub(crate) mod raft_state;
pub(crate) mod utime;

pub mod base;
#[cfg(feature = "compat")]
pub mod compat;
pub mod docs;
pub mod entry;
pub mod error;
pub mod impls;
pub mod instant;
pub mod log_id;
pub mod membership;
pub mod metrics;
pub mod network;
pub mod raft;
pub mod storage;
pub mod testing;
pub mod type_config;
pub mod vote;

#[cfg(test)]
mod feature_serde_test;

use std::fmt;

pub use anyerror;
pub use anyerror::AnyError;
pub use error::storage_error::ErrorSubject;
pub use error::storage_error::ErrorVerb;
pub use error::storage_error::StorageError;
#[allow(deprecated)]
pub use error::storage_error::StorageIOError;
pub use error::storage_error::ToStorageResult;
pub use openraft_macros::add_async_trait;
pub use type_config::AsyncRuntime;
pub use type_config::async_runtime;
#[cfg(feature = "tokio-rt")]
pub use type_config::async_runtime::tokio_impls::TokioRuntime;

pub use self::storage::LogState;
pub use self::storage::RaftLogReader;
pub use self::storage::RaftSnapshotBuilder;
pub use self::storage::Snapshot;
pub use self::storage::SnapshotMeta;
pub use self::storage::StorageHelper;
use crate::base::OptionalFeatures;
pub use crate::base::OptionalSend;
pub use crate::base::OptionalSerde;
pub use crate::base::OptionalSync;
pub use crate::change_members::ChangeMembers;
pub use crate::config::Config;
pub use crate::config::ConfigError;
pub use crate::config::SnapshotPolicy;
pub use crate::core::ServerState;
pub use crate::entry::Entry;
pub use crate::entry::EntryPayload;
pub use crate::instant::Instant;
#[cfg(feature = "tokio-rt")]
pub use crate::instant::TokioInstant;
pub use crate::log_id::LogId;
pub use crate::log_id::LogIdOptionExt;
pub use crate::log_id::LogIndexOptionExt;
pub use crate::membership::EffectiveMembership;
pub use crate::membership::Membership;
pub use crate::membership::StoredMembership;
pub use crate::metrics::RaftMetrics;
pub use crate::network::RPCTypes;
pub use crate::network::RaftNetwork;
pub use crate::network::RaftNetworkFactory;
pub use crate::node::BasicNode;
pub use crate::node::EmptyNode;
pub use crate::node::Node;
pub use crate::node::NodeId;
pub use crate::raft::Raft;
pub use crate::raft::ReadPolicy;
pub use crate::raft_state::MembershipState;
pub use crate::raft_state::RaftState;
pub use crate::raft_types::SnapshotId;
pub use crate::raft_types::SnapshotSegmentId;
pub use crate::summary::MessageSummary;
pub use crate::try_as_ref::TryAsRef;
pub use crate::type_config::RaftTypeConfig;
#[cfg(feature = "type-alias")]
pub use crate::type_config::alias;
pub use crate::vote::Vote;

/// A trait defining application-specific data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types throughout their application without having to serialize and
/// deserialize their data as it goes through Raft. Instead, applications can present their data
/// models as-is to Raft, Raft will present it to the application's `RaftLogStorage` and
/// `RaftStateMachine` impl when ready, and the application may then deal with the data directly in
/// the storage engine without having to do a preliminary deserialization.
///
/// ## Note
///
/// The trait is automatically implemented for all types that satisfy its supertraits.
pub trait AppData: OptionalFeatures + fmt::Debug + fmt::Display + 'static {}

impl<T> AppData for T where T: OptionalFeatures + fmt::Debug + fmt::Display + 'static {}

/// A trait defining application-specific response data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types for returning response data from the storage layer when an
/// entry is applied to the state machine as part of a client request (this is not used during
/// replication). This allows applications to seamlessly return application-specific data from
/// their storage layer, up through Raft, and back into their application for returning
/// data to clients.
///
/// This type must encapsulate both success and error responses, as application-specific logic
/// related to the success or failure of a client request — application-specific validation logic,
/// enforcing of data constraints, and anything of that nature — are expressly out of the realm of
/// the Raft consensus protocol.
///
/// ## Note
///
/// The trait is automatically implemented for all types that satisfy its supertraits.
pub trait AppDataResponse: OptionalFeatures + 'static {}

impl<T> AppDataResponse for T where T: OptionalFeatures + 'static {}
