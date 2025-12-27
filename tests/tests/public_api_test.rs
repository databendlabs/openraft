//! Compile-time test to verify all public API paths remain accessible from external crates.
//!
//! This test ensures that refactoring or restructuring of modules
//! does not break any existing public API paths.

#![allow(unused_imports)]
#![allow(dead_code)]

// =============================================================================
// Root-level exports from openraft
// =============================================================================

use openraft::AnyError;
use openraft::AppData;
use openraft::AppDataResponse;
use openraft::AsyncRuntime;
use openraft::BasicNode;
use openraft::ChangeMembers;
use openraft::Config;
use openraft::ConfigError;
use openraft::EffectiveMembership;
use openraft::EmptyNode;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::Instant;
use openraft::LogId;
use openraft::LogIdOptionExt;
use openraft::LogIndexOptionExt;
use openraft::LogState;
use openraft::Membership;
use openraft::MembershipState;
use openraft::MessageSummary;
use openraft::Node;
use openraft::NodeId;
use openraft::OptionalSend;
use openraft::OptionalSerde;
use openraft::OptionalSync;
use openraft::RPCTypes;
use openraft::Raft;
use openraft::RaftLogReader;
use openraft::RaftMetrics;
#[allow(deprecated)]
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::RaftSnapshotBuilder;
use openraft::RaftState;
use openraft::RaftTypeConfig;
use openraft::ReadPolicy;
use openraft::Snapshot;
use openraft::SnapshotId;
use openraft::SnapshotMeta;
use openraft::SnapshotPolicy;
use openraft::SnapshotSegmentId;
use openraft::StorageError;
use openraft::StorageHelper;
use openraft::StoredMembership;
use openraft::ToStorageResult;
use openraft::TryAsRef;
use openraft::Vote;
use openraft::WatchChangeHandle;
use openraft::WatchSender;
use openraft::add_async_trait;
use openraft::anyerror;
// =============================================================================
// async_runtime module exports
// =============================================================================
use openraft::async_runtime;
use openraft::async_runtime::AsyncRuntime as AsyncRuntime2;
use openraft::async_runtime::Instant as Instant2;
use openraft::async_runtime::Mpsc;
use openraft::async_runtime::MpscReceiver;
use openraft::async_runtime::MpscSender;
use openraft::async_runtime::MpscWeakSender;
use openraft::async_runtime::Mutex;
use openraft::async_runtime::Oneshot;
use openraft::async_runtime::OneshotSender;
use openraft::async_runtime::RecvError;
use openraft::async_runtime::SendError;
use openraft::async_runtime::TokioInstant;
use openraft::async_runtime::TryRecvError;
use openraft::async_runtime::Watch;
use openraft::async_runtime::WatchReceiver;
use openraft::async_runtime::WatchSender as WatchSender2;
use openraft::async_runtime::instant;
use openraft::async_runtime::instant::Instant as InstantTrait;
use openraft::async_runtime::mpsc;
use openraft::async_runtime::mpsc::Mpsc as MpscTrait;
use openraft::async_runtime::mpsc::MpscReceiver as MpscReceiverTrait;
use openraft::async_runtime::mpsc::MpscSender as MpscSenderTrait;
use openraft::async_runtime::mpsc::MpscWeakSender as MpscWeakSenderTrait;
use openraft::async_runtime::mpsc::SendError as MpscSendError;
use openraft::async_runtime::mpsc::TryRecvError as MpscTryRecvError;
use openraft::async_runtime::mutex;
use openraft::async_runtime::mutex::Mutex as MutexTrait;
use openraft::async_runtime::oneshot;
use openraft::async_runtime::oneshot::Oneshot as OneshotTrait;
use openraft::async_runtime::oneshot::OneshotSender as OneshotSenderTrait;
use openraft::async_runtime::watch;
use openraft::async_runtime::watch::RecvError as WatchRecvError;
use openraft::async_runtime::watch::SendError as WatchSendError;
use openraft::async_runtime::watch::Watch as WatchTrait;
use openraft::async_runtime::watch::WatchReceiver as WatchReceiverTrait;
use openraft::async_runtime::watch::WatchSender as WatchSenderTrait;
// =============================================================================
// base module exports
// =============================================================================
use openraft::base::OptionalFeatures;
use openraft::base::OptionalSend as BaseOptionalSend;
use openraft::base::OptionalSerde as BaseOptionalSerde;
use openraft::base::OptionalSync as BaseOptionalSync;
// Note: config module is private; items are re-exported at root level

// =============================================================================
// entry module exports
// =============================================================================
use openraft::entry::Entry as EntryModule;
use openraft::entry::EntryPayload as EntryPayloadModule;
use openraft::entry::RaftEntry;
use openraft::entry::RaftPayload;
// =============================================================================
// error module exports
// =============================================================================
use openraft::error::AllowNextRevertError;
use openraft::error::ChangeMembershipError;
use openraft::error::ClientWriteError;
use openraft::error::EmptyMembership;
use openraft::error::Fatal;
use openraft::error::ForwardToLeader;
use openraft::error::InProgress;
use openraft::error::Infallible;
use openraft::error::InitializeError;
use openraft::error::InstallSnapshotError;
use openraft::error::InvalidStateMachineType;
use openraft::error::LeaderChanged;
use openraft::error::LearnerNotFound;
use openraft::error::LinearizableReadError;
use openraft::error::MembershipError;
use openraft::error::NetworkError;
use openraft::error::NoForward;
use openraft::error::NodeNotFound;
use openraft::error::NotAllowed;
use openraft::error::NotInMembers;
use openraft::error::Operation;
use openraft::error::QuorumNotEnough;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::RemoteError;
use openraft::error::ReplicationClosed;
use openraft::error::SnapshotMismatch;
use openraft::error::StreamingError;
use openraft::error::Timeout;
use openraft::error::Unreachable;
use openraft::error::decompose;
// =============================================================================
// impls module exports
// =============================================================================
use openraft::impls::BasicNode as ImplsBasicNode;
use openraft::impls::EmptyNode as ImplsEmptyNode;
use openraft::impls::Entry as ImplsEntry;
use openraft::impls::LogId as ImplsLogId;
use openraft::impls::OneshotResponder;
use openraft::impls::ProgressResponder;
use openraft::impls::TokioRuntime;
use openraft::impls::Vote as ImplsVote;
use openraft::impls::leader_id_adv;
use openraft::impls::leader_id_adv::LeaderId as LeaderIdAdv;
use openraft::impls::leader_id_std;
use openraft::impls::leader_id_std::LeaderId as LeaderIdStd;
// =============================================================================
// log_id module exports
// =============================================================================
use openraft::log_id::LogId as LogIdModule;
use openraft::log_id::LogIdOptionExt as LogIdOptionExtModule;
use openraft::log_id::LogIndexOptionExt as LogIndexOptionExtModule;
// =============================================================================
// membership module exports
// =============================================================================
use openraft::membership::EffectiveMembership as EffectiveMembershipModule;
use openraft::membership::Membership as MembershipModule;
use openraft::membership::StoredMembership as StoredMembershipModule;
// =============================================================================
// metrics module exports
// =============================================================================
use openraft::metrics::RaftMetrics as RaftMetricsModule;
use openraft::metrics::Wait;
use openraft::metrics::WaitError;
// =============================================================================
// network module exports
// =============================================================================
use openraft::network::Backoff;
use openraft::network::RPCOption;
use openraft::network::RPCTypes as RPCTypesModule;
#[allow(deprecated)]
use openraft::network::RaftNetwork as RaftNetworkModule;
use openraft::network::RaftNetworkFactory as RaftNetworkFactoryModule;
use openraft::network::v2;
use openraft::network::v2::RaftNetworkV2;
// =============================================================================
// raft module exports
// =============================================================================
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::ClientWriteResponse;
use openraft::raft::ClientWriteResult;
use openraft::raft::FlushPoint;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::Leader;
use openraft::raft::Raft as RaftModule;
use openraft::raft::ReadPolicy as ReadPolicyModule;
use openraft::raft::RuntimeConfigHandle;
use openraft::raft::SnapshotResponse;
use openraft::raft::StreamAppendError;
use openraft::raft::StreamAppendResult;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::raft::WatchChangeHandle as WatchChangeHandleModule;
use openraft::raft::WriteRequest;
use openraft::raft::WriteResponse;
use openraft::raft::WriteResult;
use openraft::raft::linearizable_read;
use openraft::raft::linearizable_read::Linearizer;
use openraft::raft::responder;
use openraft::raft::responder::Responder;
use openraft::raft::trigger;
use openraft::raft::trigger::Trigger;
// =============================================================================
// storage module exports
// =============================================================================
use openraft::storage::ApplyResponder;
use openraft::storage::EntryResponder;
use openraft::storage::IOFlushed;
use openraft::storage::LeaderBoundedStreamError;
use openraft::storage::LeaderBoundedStreamResult;
use openraft::storage::LogApplied;
use openraft::storage::LogState as LogStateModule;
use openraft::storage::RaftLogReader as RaftLogReaderModule;
use openraft::storage::RaftLogReaderExt;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftLogStorageExt;
use openraft::storage::RaftSnapshotBuilder as RaftSnapshotBuilderModule;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot as SnapshotModule;
use openraft::storage::SnapshotMeta as SnapshotMetaModule;
use openraft::storage::SnapshotSignature;
use openraft::storage::StorageHelper as StorageHelperModule;
// =============================================================================
// testing module exports
// =============================================================================
use openraft::testing;
use openraft::testing::log::StoreBuilder as LogStoreBuilder;
use openraft::testing::log::Suite as LogSuite;
// =============================================================================
// type_config module exports
// =============================================================================
use openraft::type_config::AsyncRuntime as TypeConfigAsyncRuntime;
use openraft::type_config::OneshotSender as TypeConfigOneshotSender;
use openraft::type_config::RaftTypeConfig as TypeConfigRaftTypeConfig;
use openraft::type_config::async_runtime as type_config_async_runtime;
// =============================================================================
// vote module exports
// =============================================================================
use openraft::vote::RaftLeaderId;
use openraft::vote::RaftTerm;
use openraft::vote::Vote as VoteModule;

// =============================================================================
// Macros
// =============================================================================

// Test that declare_raft_types macro is accessible
// openraft::declare_raft_types! is tested implicitly by usage in examples

#[test]
fn test_public_api_accessible() {
    // This test just needs to compile to verify all paths are publicly accessible
}
