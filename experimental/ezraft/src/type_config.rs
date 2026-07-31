//! Type configuration for EzRaft
//!
//! This module provides the `OpenRaftTypes` wrapper that implements
//! OpenRaft's `RaftTypeConfig` with sensible defaults for any [`EzApp`].

use std::marker::PhantomData;

use openraft::BasicNode;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::Vote;
use openraft::impls::InlineBatch;
use openraft::impls::OneshotResponder;
use openraft::impls::leader_id_std::LeaderId;

use crate::app::EzApp;

/// Wrapper type that implements `RaftTypeConfig` for any `T: EzApp`
///
/// This provides all the default implementations needed for OpenRaft.
pub struct OpenRaftTypes<T>
where T: EzApp
{
    _phantom: PhantomData<T>,
}

impl<T> Copy for OpenRaftTypes<T> where T: EzApp {}

impl<T> Clone for OpenRaftTypes<T>
where T: EzApp
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Default for OpenRaftTypes<T>
where T: EzApp
{
    fn default() -> Self {
        Self { _phantom: PhantomData }
    }
}

impl<T> PartialEq for OpenRaftTypes<T>
where T: EzApp
{
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T> Eq for OpenRaftTypes<T> where T: EzApp {}

impl<T> PartialOrd for OpenRaftTypes<T>
where T: EzApp
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for OpenRaftTypes<T>
where T: EzApp
{
    fn cmp(&self, _other: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl<T> std::fmt::Debug for OpenRaftTypes<T>
where T: EzApp
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("OpenRaftTypes").finish()
    }
}

impl<T> RaftTypeConfig for OpenRaftTypes<T>
where T: EzApp
{
    type D = T::Request;
    // `None` answers framework-generated entries (blank, membership); every user request is
    // answered with `Some` by the state machine.
    type R = Option<T::Response>;
    type NodeId = u64;
    type Node = BasicNode;
    type Term = u64;
    type LeaderId = EzLeaderId;
    type Vote = Vote<EzLeaderId>;
    type Entry = crate::entry::EzEntry<T>;
    type AsyncRuntime = openraft::TokioRuntime;
    type Responder<X>
        = OneshotResponder<Self, X>
    where X: OptionalSend + 'static;
    type Batch<X>
        = InlineBatch<X>
    where X: OptionalSend + 'static;
    type ErrorSource = openraft::AnyError;
}

/// Leader ID type: standard Raft, at most one leader per term
pub type EzLeaderId = LeaderId<u64, u64>;

/// Type alias for the Raft vote
pub type EzVote = Vote<EzLeaderId>;
