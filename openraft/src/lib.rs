#![doc = include_str!("../../README.md")]
#![cfg_attr(feature = "bt", feature(backtrace))]

//! # Feature flags
//!
//! - `bt`: Enable backtrace: generate backtrace for errors. This requires a unstable feature `backtrace` thus it can
//!   not be used with stable rust, unless explicity allowing using unstable features in stable rust with
//!   `RUSTC_BOOTSTRAP=1`.

mod config;
mod core;
mod defensive;
mod entry;
mod membership;
mod node;
mod raft_types;
mod replication;
mod storage_error;
mod store_ext;
mod store_wrapper;
mod summary;
mod vote;

mod engine;
pub mod error;
mod leader;
pub mod metrics;
pub mod network;
pub mod raft;
mod raft_state;
mod runtime;
pub mod storage;
pub mod testing;
pub mod timer;
pub mod versioned;

#[cfg(test)] mod raft_state_test;

use std::fmt::Debug;
use std::marker::PhantomData;

pub use anyerror;
pub use anyerror::AnyError;
pub use async_trait;
pub use metrics::ReplicationTargetMetrics;
use store_ext::LogReaderExt;
use store_ext::SnapshotBuilderExt;

pub use crate::config::Config;
pub use crate::config::ConfigError;
pub use crate::config::RemoveReplicationPolicy;
pub use crate::config::SnapshotPolicy;
pub use crate::core::ServerState;
pub use crate::defensive::DefensiveCheck;
pub use crate::defensive::DefensiveCheckBase;
pub use crate::entry::Entry;
pub use crate::entry::EntryPayload;
pub use crate::entry::RaftPayload;
pub use crate::membership::EffectiveMembership;
pub use crate::membership::Membership;
pub use crate::membership::MembershipState;
pub use crate::metrics::RaftMetrics;
pub use crate::network::RPCTypes;
pub use crate::network::RaftNetwork;
pub use crate::network::RaftNetworkFactory;
pub use crate::node::Node;
pub use crate::node::NodeId;
pub use crate::raft::ChangeMembers;
pub use crate::raft::Raft;
pub use crate::raft::RaftTypeConfig;
pub use crate::raft_state::RaftState;
pub use crate::raft_types::LogId;
pub use crate::raft_types::LogIdOptionExt;
pub(crate) use crate::raft_types::MetricsChangeFlags;
pub use crate::raft_types::SnapshotId;
pub use crate::raft_types::SnapshotSegmentId;
pub use crate::raft_types::StateMachineChanges;
pub use crate::raft_types::Update;
pub use crate::storage::RaftLogReader;
pub use crate::storage::RaftSnapshotBuilder;
pub use crate::storage::RaftStorage;
pub use crate::storage::RaftStorageDebug;
pub use crate::storage::SnapshotMeta;
pub use crate::storage_error::DefensiveError;
pub use crate::storage_error::ErrorSubject;
pub use crate::storage_error::ErrorVerb;
pub use crate::storage_error::StorageError;
pub use crate::storage_error::StorageIOError;
pub use crate::storage_error::ToStorageResult;
pub use crate::storage_error::Violation;
pub use crate::store_ext::StoreExt;
pub use crate::store_wrapper::Wrapper;
pub use crate::summary::MessageSummary;
pub use crate::vote::LeaderId;
pub use crate::vote::Vote;

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
#[cfg(feature = "serde")]
pub trait AppData: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static {}
#[cfg(feature = "serde")]
impl<T> AppData for T where T: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static {}

#[cfg(not(feature = "serde"))]
pub trait AppData: Clone + Send + Sync + 'static {}

#[cfg(not(feature = "serde"))]
impl<T> AppData for T where T: Clone + Send + Sync + 'static {}

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
#[cfg(feature = "serde")]
pub trait AppDataResponse: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static {}

#[cfg(feature = "serde")]
impl<T> AppDataResponse for T where T: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static {}

#[cfg(not(feature = "serde"))]
pub trait AppDataResponse: Clone + Send + Sync + 'static {}

#[cfg(not(feature = "serde"))]
impl<T> AppDataResponse for T where T: Clone + Send + Sync + 'static {}

/// This struct is only used for configuration or testing.
///
/// ## Note
///
/// (Un)Implemented RaftStorage trait.
pub struct DummyStorage<C: RaftTypeConfig> {
    c: PhantomData<C>,
}

#[async_trait::async_trait]
impl<C: RaftTypeConfig> RaftStorage<C> for DummyStorage<C>
where C: RaftTypeConfig
{
    type SnapshotData = <<C as RaftTypeConfig>::S as RaftStorage<C>>::SnapshotData;

    type LogReader = LogReaderExt<C, C::S>;

    type SnapshotBuilder = SnapshotBuilderExt<C, C::S>;

    fn save_vote<'life0, 'life1, 'async_trait>(
        &'life0 mut self,
        _vote: &'life1 crate::Vote<<C as RaftTypeConfig>::NodeId>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::StorageError<<C as RaftTypeConfig>::NodeId>>>
                + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn read_vote<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::Vote<<C as RaftTypeConfig>::NodeId>>,
                        crate::StorageError<<C as RaftTypeConfig>::NodeId>,
                    >,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn get_log_reader<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Self::LogReader> + std::marker::Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn append_to_log<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 mut self,
        _entries: &'life1 [&'life2 crate::Entry<C>],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::StorageError<<C as RaftTypeConfig>::NodeId>>>
                + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn delete_conflict_logs_since<'life0, 'async_trait>(
        &'life0 mut self,
        _log_id: crate::LogId<<C as RaftTypeConfig>::NodeId>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::StorageError<<C as RaftTypeConfig>::NodeId>>>
                + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn purge_logs_upto<'life0, 'async_trait>(
        &'life0 mut self,
        _log_id: crate::LogId<<C as RaftTypeConfig>::NodeId>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::StorageError<<C as RaftTypeConfig>::NodeId>>>
                + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn last_applied_state<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        (
                            Option<crate::LogId<<C as RaftTypeConfig>::NodeId>>,
                            crate::EffectiveMembership<<C as RaftTypeConfig>::NodeId>,
                        ),
                        crate::StorageError<<C as RaftTypeConfig>::NodeId>,
                    >,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn apply_to_state_machine<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 mut self,
        _entries: &'life1 [&'life2 crate::Entry<C>],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<<C as RaftTypeConfig>::R>, crate::StorageError<<C as RaftTypeConfig>::NodeId>>,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn get_snapshot_builder<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Self::SnapshotBuilder> + std::marker::Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn begin_receiving_snapshot<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Box<Self::SnapshotData>, crate::StorageError<<C as RaftTypeConfig>::NodeId>>,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn install_snapshot<'life0, 'life1, 'async_trait>(
        &'life0 mut self,
        _meta: &'life1 crate::SnapshotMeta<<C as RaftTypeConfig>::NodeId>,
        _snapshot: Box<Self::SnapshotData>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<crate::StateMachineChanges<C>, crate::StorageError<<C as RaftTypeConfig>::NodeId>>,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn get_current_snapshot<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::storage::Snapshot<<C as RaftTypeConfig>::NodeId, Self::SnapshotData>>,
                        crate::StorageError<<C as RaftTypeConfig>::NodeId>,
                    >,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }
}

#[async_trait::async_trait]
impl<C: RaftTypeConfig> RaftLogReader<C> for DummyStorage<C> {
    fn get_log_state<'life0, 'async_trait>(
        &'life0 mut self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<crate::storage::LogState<C>, crate::StorageError<<C as RaftTypeConfig>::NodeId>>,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn try_get_log_entries<'life0, 'async_trait, RB>(
        &'life0 mut self,
        _range: RB,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<crate::Entry<C>>, crate::StorageError<<C as RaftTypeConfig>::NodeId>>,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        RB: 'async_trait + std::ops::RangeBounds<u64> + Clone + std::fmt::Debug + Send + Sync,
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }
}

/// This struct is only used for configuration or testing.
///
/// ## Note
///
/// (Un)Implemented RaftNetwork trait.
pub struct DummyNetwork<C: RaftTypeConfig>
where
    C::D: Debug,
    C::R: Debug,
    //S: Default + Clone,
{
    dummy_use_c: PhantomData<C>,
}

#[async_trait::async_trait]
impl<C: RaftTypeConfig> RaftNetwork<C> for DummyNetwork<C>
where
    C::D: Debug,
    C::R: Debug,
    //S: Default + Clone,
{
    fn send_append_entries<'life0, 'async_trait>(
        &'life0 mut self,
        _rpc: crate::raft::AppendEntriesRequest<C>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::raft::AppendEntriesResponse<<C as RaftTypeConfig>::NodeId>,
                        crate::error::RPCError<
                            <C as RaftTypeConfig>::NodeId,
                            crate::error::AppendEntriesError<<C as RaftTypeConfig>::NodeId>,
                        >,
                    >,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn send_install_snapshot<'life0, 'async_trait>(
        &'life0 mut self,
        _rpc: crate::raft::InstallSnapshotRequest<C>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::raft::InstallSnapshotResponse<<C as RaftTypeConfig>::NodeId>,
                        crate::error::RPCError<
                            <C as RaftTypeConfig>::NodeId,
                            crate::error::InstallSnapshotError<<C as RaftTypeConfig>::NodeId>,
                        >,
                    >,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn send_vote<'life0, 'async_trait>(
        &'life0 mut self,
        _rpc: crate::raft::VoteRequest<<C as RaftTypeConfig>::NodeId>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::raft::VoteResponse<<C as RaftTypeConfig>::NodeId>,
                        crate::error::RPCError<
                            <C as RaftTypeConfig>::NodeId,
                            crate::error::VoteError<<C as RaftTypeConfig>::NodeId>,
                        >,
                    >,
                > + std::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }
}
