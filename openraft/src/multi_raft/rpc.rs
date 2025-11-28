//! Multi-Raft RPC types for routing requests to the correct Raft group.
//!
//! This module provides wrapper types that add group routing information to standard
//! Raft RPC requests and responses, enabling multiple Raft groups to share the same
//! network connection.

use std::fmt;

use super::MultiRaftTypeConfig;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::InstallSnapshotRequest;
use crate::raft::InstallSnapshotResponse;
use crate::raft::SnapshotResponse;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;

/// A wrapper that adds group routing information to any RPC request.
///
/// In a Multi-Raft deployment, multiple Raft groups can share the same network connection.
/// `GroupedRequest` wraps a standard Raft RPC request with the target group's identifier,
/// allowing the receiving node to route the request to the correct Raft instance.
///
/// ## Type Parameters
///
/// - `C`: The [`MultiRaftTypeConfig`] that defines `GroupId` and other types
/// - `R`: The inner request type (e.g., `AppendEntriesRequest<C>`)
///
/// ## Example
///
/// ```ignore
/// use openraft::multi_raft::GroupedRequest;
///
/// // Wrap an AppendEntries request with group information
/// let grouped = GroupedRequest {
///     group_id: "region_1".to_string(),
///     request: append_entries_request,
/// };
///
/// // Send to the target node
/// let response = network.send_grouped_append_entries(grouped).await?;
/// ```
#[derive(Clone)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(serialize = "R: serde::Serialize", deserialize = "R: serde::de::DeserializeOwned"))
)]
pub struct GroupedRequest<C, R>
where C: MultiRaftTypeConfig
{
    /// The target Raft group's identifier.
    pub group_id: C::GroupId,

    /// The inner RPC request.
    pub request: R,
}

impl<C, R> GroupedRequest<C, R>
where C: MultiRaftTypeConfig
{
    /// Creates a new grouped request.
    pub fn new(group_id: C::GroupId, request: R) -> Self {
        Self { group_id, request }
    }

    /// Extracts the inner request, discarding the group information.
    pub fn into_inner(self) -> R {
        self.request
    }

    /// Returns a reference to the group ID.
    pub fn group_id(&self) -> &C::GroupId {
        &self.group_id
    }

    /// Returns a reference to the inner request.
    pub fn request(&self) -> &R {
        &self.request
    }

    /// Maps the inner request to a different type.
    pub fn map<R2, F>(self, f: F) -> GroupedRequest<C, R2>
    where F: FnOnce(R) -> R2 {
        GroupedRequest {
            group_id: self.group_id,
            request: f(self.request),
        }
    }
}

impl<C, R> fmt::Debug for GroupedRequest<C, R>
where
    C: MultiRaftTypeConfig,
    R: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupedRequest")
            .field("group_id", &self.group_id)
            .field("request", &self.request)
            .finish()
    }
}

impl<C, R> fmt::Display for GroupedRequest<C, R>
where
    C: MultiRaftTypeConfig,
    R: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[group={}] {}", self.group_id, self.request)
    }
}

/// A wrapper that adds group routing information to any RPC response.
///
/// Similar to [`GroupedRequest`], this wraps a response with the originating group's
/// identifier. This is useful for multiplexed connections where responses from different
/// groups arrive on the same channel.
#[derive(Clone)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(serialize = "R: serde::Serialize", deserialize = "R: serde::de::DeserializeOwned"))
)]
pub struct GroupedResponse<C, R>
where C: MultiRaftTypeConfig
{
    /// The originating Raft group's identifier.
    pub group_id: C::GroupId,

    /// The inner RPC response.
    pub response: R,
}

impl<C, R> GroupedResponse<C, R>
where C: MultiRaftTypeConfig
{
    /// Creates a new grouped response.
    pub fn new(group_id: C::GroupId, response: R) -> Self {
        Self { group_id, response }
    }

    /// Extracts the inner response, discarding the group information.
    pub fn into_inner(self) -> R {
        self.response
    }

    /// Returns a reference to the group ID.
    pub fn group_id(&self) -> &C::GroupId {
        &self.group_id
    }

    /// Returns a reference to the inner response.
    pub fn response(&self) -> &R {
        &self.response
    }
}

impl<C, R> fmt::Debug for GroupedResponse<C, R>
where
    C: MultiRaftTypeConfig,
    R: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupedResponse")
            .field("group_id", &self.group_id)
            .field("response", &self.response)
            .finish()
    }
}

impl<C, R> fmt::Display for GroupedResponse<C, R>
where
    C: MultiRaftTypeConfig,
    R: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[group={}] {}", self.group_id, self.response)
    }
}

// ============================================================================
// Type Aliases for Common Grouped Request/Response Types
// ============================================================================

/// AppendEntries request with group routing information.
pub type GroupedAppendEntriesRequest<C> = GroupedRequest<C, AppendEntriesRequest<C>>;

/// AppendEntries response with group routing information.
pub type GroupedAppendEntriesResponse<C> = GroupedResponse<C, AppendEntriesResponse<C>>;

/// Vote request with group routing information.
pub type GroupedVoteRequest<C> = GroupedRequest<C, VoteRequest<C>>;

/// Vote response with group routing information.
pub type GroupedVoteResponse<C> = GroupedResponse<C, VoteResponse<C>>;

/// InstallSnapshot request with group routing information.
pub type GroupedInstallSnapshotRequest<C> = GroupedRequest<C, InstallSnapshotRequest<C>>;

/// InstallSnapshot response with group routing information.
pub type GroupedInstallSnapshotResponse<C> = GroupedResponse<C, InstallSnapshotResponse<C>>;

/// Snapshot response with group routing information.
pub type GroupedSnapshotResponse<C> = GroupedResponse<C, SnapshotResponse<C>>;

// ============================================================================
// Helper methods for creating grouped requests
// ============================================================================

impl<C: MultiRaftTypeConfig> GroupedAppendEntriesRequest<C> {
    /// Creates a grouped AppendEntries request.
    pub fn append_entries(group_id: C::GroupId, request: AppendEntriesRequest<C>) -> Self {
        Self::new(group_id, request)
    }
}

impl<C: MultiRaftTypeConfig> GroupedVoteRequest<C> {
    /// Creates a grouped Vote request.
    pub fn vote(group_id: C::GroupId, request: VoteRequest<C>) -> Self {
        Self::new(group_id, request)
    }
}

impl<C: MultiRaftTypeConfig> GroupedInstallSnapshotRequest<C> {
    /// Creates a grouped InstallSnapshot request.
    pub fn install_snapshot(group_id: C::GroupId, request: InstallSnapshotRequest<C>) -> Self {
        Self::new(group_id, request)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::EmptyNode;
    use crate::OptionalSend;
    use crate::RaftTypeConfig;

    #[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
    struct TestConfig;

    impl RaftTypeConfig for TestConfig {
        type D = String;
        type R = String;
        type NodeId = u64;
        type Node = EmptyNode;
        type Term = u64;
        type LeaderId = crate::impls::leader_id_adv::LeaderId<Self>;
        type Vote = crate::impls::Vote<Self>;
        type Entry = crate::impls::Entry<Self>;
        type SnapshotData = Cursor<Vec<u8>>;
        type AsyncRuntime = crate::impls::TokioRuntime;
        type Responder<T>
            = crate::impls::OneshotResponder<Self, T>
        where T: OptionalSend + 'static;
    }

    impl MultiRaftTypeConfig for TestConfig {
        type GroupId = String;
    }

    #[test]
    fn test_grouped_request_new() {
        // Vote::new takes (term, node_id)
        let vote = crate::impls::Vote::<TestConfig>::new(1, 1);
        let inner = VoteRequest::<TestConfig>::new(vote, None);
        let grouped = GroupedRequest::<TestConfig, _>::new("group1".to_string(), inner.clone());

        assert_eq!(grouped.group_id(), "group1");
        assert_eq!(grouped.request().vote, inner.vote);
    }

    #[test]
    fn test_grouped_request_into_inner() {
        let vote = crate::impls::Vote::<TestConfig>::new(1, 1);
        let inner = VoteRequest::<TestConfig>::new(vote.clone(), None);
        let grouped = GroupedRequest::<TestConfig, _>::new("group1".to_string(), inner);

        let extracted = grouped.into_inner();
        assert_eq!(extracted.vote, vote);
    }

    #[test]
    fn test_grouped_request_map() {
        let grouped = GroupedRequest::<TestConfig, i32>::new("group1".to_string(), 42);
        let mapped = grouped.map(|x| x.to_string());

        assert_eq!(mapped.group_id(), "group1");
        assert_eq!(mapped.request(), "42");
    }

    #[test]
    fn test_grouped_request_debug() {
        let grouped = GroupedRequest::<TestConfig, i32>::new("group1".to_string(), 42);
        let debug_str = format!("{:?}", grouped);

        assert!(debug_str.contains("GroupedRequest"));
        assert!(debug_str.contains("group1"));
        assert!(debug_str.contains("42"));
    }

    #[test]
    fn test_grouped_request_display() {
        let grouped = GroupedRequest::<TestConfig, i32>::new("group1".to_string(), 42);
        let display_str = format!("{}", grouped);

        assert_eq!(display_str, "[group=group1] 42");
    }

    #[test]
    fn test_grouped_response_new() {
        let response = GroupedResponse::<TestConfig, String>::new("group1".to_string(), "success".to_string());

        assert_eq!(response.group_id(), "group1");
        assert_eq!(response.response(), "success");
    }

    #[test]
    fn test_grouped_response_into_inner() {
        let response = GroupedResponse::<TestConfig, String>::new("group1".to_string(), "success".to_string());

        let inner = response.into_inner();
        assert_eq!(inner, "success");
    }

    #[test]
    fn test_type_aliases() {
        // Just verify the type aliases compile correctly
        fn _check_aliases() {
            let _: Option<GroupedAppendEntriesRequest<TestConfig>> = None;
            let _: Option<GroupedAppendEntriesResponse<TestConfig>> = None;
            let _: Option<GroupedVoteRequest<TestConfig>> = None;
            let _: Option<GroupedVoteResponse<TestConfig>> = None;
            let _: Option<GroupedInstallSnapshotRequest<TestConfig>> = None;
            let _: Option<GroupedInstallSnapshotResponse<TestConfig>> = None;
            let _: Option<GroupedSnapshotResponse<TestConfig>> = None;
        }
    }
}
