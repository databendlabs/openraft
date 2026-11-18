//! Test helpers shared by the unit tests of this crate.

use openraft::impls::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::testing::log_id;

openraft::declare_raft_types!(pub(crate) TestConfig);

/// An empty AppendEntries request whose `prev_log_id` has `index`.
///
/// A successful reply makes it return `appended(index)`, which tells the groups' results apart.
pub(crate) fn heartbeat(index: u64) -> AppendEntriesRequest<TestConfig> {
    AppendEntriesRequest {
        vote: Vote::new_committed(1, 0),
        prev_log_id: Some(log_id::<TestConfig>(1, 0, index)),
        entries: vec![],
        leader_commit: None,
    }
}

/// The result of a successful `heartbeat(index)`.
pub(crate) fn appended(index: u64) -> AppendResult<TestConfig> {
    Ok(Some(log_id::<TestConfig>(1, 0, index)))
}
