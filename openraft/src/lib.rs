#![doc = include_str!("lib_readme.md")]
#![doc = include_str!("docs/docs.md")]
#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![cfg_attr(feature = "bench", feature(test))]
// TODO: `clippy::result-large-err`: StorageError is 136 bytes, try to reduce the size.
#![allow(clippy::bool_assert_comparison, clippy::type_complexity, clippy::result_large_err)]
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

mod change_members;
mod config;
mod core;
mod defensive;
mod membership;
mod node;
mod progress;
mod quorum;
mod raft_types;
mod replication;
mod storage_error;
mod summary;
mod vote;

#[cfg(feature = "compat")] pub mod compat;
#[cfg(feature = "compat-07")] pub use or07;

pub mod async_runtime;
pub mod entry;
pub mod error;
pub mod instant;
pub mod log_id;
pub mod metrics;
pub mod network;
pub mod raft;
pub mod storage;
pub mod testing;
pub mod timer;
pub mod type_config;

pub(crate) mod engine;
pub(crate) mod log_id_range;
pub(crate) mod utime;

mod display_ext;

pub mod docs;
mod internal_server_state;
mod leader;
pub(crate) mod raft_state;
mod runtime;
mod try_as_ref;

#[cfg(test)] mod feature_serde_test;

pub use anyerror;
pub use anyerror::AnyError;
pub use async_trait;
pub use macros::add_async_trait;
pub use network::RPCTypes;
pub use network::RaftNetwork;
pub use network::RaftNetworkFactory;
pub use type_config::RaftTypeConfig;

pub use crate::async_runtime::AsyncRuntime;
pub use crate::async_runtime::TokioRuntime;
pub use crate::change_members::ChangeMembers;
pub use crate::config::Config;
pub use crate::config::ConfigError;
pub use crate::config::SnapshotPolicy;
pub use crate::core::ServerState;
pub use crate::entry::Entry;
pub use crate::entry::EntryPayload;
pub use crate::instant::Instant;
pub use crate::instant::TokioInstant;
pub use crate::log_id::LogId;
pub use crate::log_id::LogIdOptionExt;
pub use crate::log_id::LogIndexOptionExt;
pub use crate::log_id::RaftLogId;
pub use crate::membership::EffectiveMembership;
pub use crate::membership::Membership;
pub use crate::membership::StoredMembership;
pub use crate::metrics::RaftMetrics;
pub use crate::node::BasicNode;
pub use crate::node::EmptyNode;
pub use crate::node::Node;
pub use crate::node::NodeId;
pub use crate::raft::Raft;
pub use crate::raft_state::MembershipState;
pub use crate::raft_state::RaftState;
pub use crate::raft_types::SnapshotId;
pub use crate::raft_types::SnapshotSegmentId;
pub use crate::storage::LogState;
pub use crate::storage::RaftLogReader;
pub use crate::storage::RaftSnapshotBuilder;
pub use crate::storage::RaftStorage;
pub use crate::storage::Snapshot;
pub use crate::storage::SnapshotMeta;
pub use crate::storage::StorageHelper;
pub use crate::storage_error::DefensiveError;
pub use crate::storage_error::ErrorSubject;
pub use crate::storage_error::ErrorVerb;
pub use crate::storage_error::StorageError;
pub use crate::storage_error::StorageIOError;
pub use crate::storage_error::ToStorageResult;
pub use crate::storage_error::Violation;
pub use crate::summary::MessageSummary;
pub use crate::try_as_ref::TryAsRef;
pub use crate::vote::CommittedLeaderId;
pub use crate::vote::LeaderId;
pub use crate::vote::Vote;

#[cfg(feature = "serde")]
#[doc(hidden)]
pub trait OptionalSerde: serde::Serialize + for<'a> serde::Deserialize<'a> {}

#[cfg(feature = "serde")]
impl<T> OptionalSerde for T where T: serde::Serialize + for<'a> serde::Deserialize<'a> {}

#[cfg(not(feature = "serde"))]
#[doc(hidden)]
pub trait OptionalSerde {}

#[cfg(not(feature = "serde"))]
impl<T> OptionalSerde for T {}

#[cfg(feature = "singlethreaded")]
pub trait OptionalSend {}

#[cfg(feature = "singlethreaded")]
pub trait OptionalSync {}

#[cfg(feature = "singlethreaded")]
impl<T: ?Sized> OptionalSend for T {}

#[cfg(feature = "singlethreaded")]
impl<T: ?Sized> OptionalSync for T {}

#[cfg(not(feature = "singlethreaded"))]
pub trait OptionalSend: Send {}

#[cfg(not(feature = "singlethreaded"))]
pub trait OptionalSync: Sync {}

#[cfg(not(feature = "singlethreaded"))]
impl<T: Send + ?Sized> OptionalSend for T {}

#[cfg(not(feature = "singlethreaded"))]
impl<T: Sync + ?Sized> OptionalSync for T {}

/// A trait defining application specific data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types throughout their application without having to serialize and
/// deserialize their data as it goes through Raft. Instead, applications can present their data
/// models as-is to Raft, Raft will present it to the application's `RaftStorage` impl when ready,
/// and the application may then deal with the data directly in the storage engine without having
/// to do a preliminary deserialization.
///
/// ## Note
///
/// The trait is automatically implemented for all types which satisfy its supertraits.
pub trait AppData: OptionalSend + OptionalSync + 'static + OptionalSerde {}

impl<T> AppData for T where T: OptionalSend + OptionalSync + 'static + OptionalSerde {}

/// A trait defining application specific response data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types for returning response data from the storage layer when an
/// entry is applied to the state machine as part of a client request (this is not used during
/// replication). This allows applications to seamlessly return application specific data from
/// their storage layer, up through Raft, and back into their application for returning
/// data to clients.
///
/// This type must encapsulate both success and error responses, as application specific logic
/// related to the success or failure of a client request — application specific validation logic,
/// enforcing of data constraints, and anything of that nature — are expressly out of the realm of
/// the Raft consensus protocol.
///
/// ## Note
///
/// The trait is automatically implemented for all types which satisfy its supertraits.
pub trait AppDataResponse: OptionalSend + OptionalSync + 'static + OptionalSerde {}

impl<T> AppDataResponse for T where T: OptionalSend + OptionalSync + 'static + OptionalSerde {}
