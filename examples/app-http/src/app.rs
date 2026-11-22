use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use openraft::NodeInfo;
use openraft::Raft;
use openraft::RaftMetrics;
use openraft::RaftTypeConfig;
use openraft::ReadPolicy;
use openraft::alias::LogIdOf;
use openraft::alias::MembershipEditOf;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ClientWriteError;
use openraft::errors::Infallible;
use openraft::errors::InitializeError;
use openraft::errors::LinearizableReadError;
use openraft::errors::decompose::DecomposeResult;
use openraft::impls::ChangeMembers;
use openraft::macros::since;
use openraft::membership::RaftMembershipEditOps;
use openraft::raft::ClientWriteResponse;
use openraft::raft::linearizable_read::LinearizeState;
use openraft::raft::linearizable_read::Linearizer;
use openraft::storage::RaftStateMachine;
use openraft::vote::RaftLeaderId;
use openraft::vote::RaftVote;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::time::Instant;

use crate::Client;
use crate::FollowerReadError;
use crate::Readiness;

/// How long a follower-read endpoint spends on one read.
///
/// Asking the leader to authorize the read, following redirects to a newer leader, and waiting for
/// the local state machine to apply the authorized log ID share this budget. It is shorter than
/// the ten-second budget of a [`Client`] operation, so the client receives the node's error
/// instead of its own timeout.
#[since(version = "0.10.2")]
pub const FOLLOWER_READ_TIMEOUT: Duration = Duration::from_secs(5);

pub struct App<C, SM, D>
where
    C: RaftTypeConfig<Node = NodeInfo>,
    SM: RaftStateMachine<C>,
{
    pub id: C::NodeId,
    pub api_addr: String,
    pub raft_addr: String,
    pub raft: Raft<C, SM>,
    pub data: D,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddLearnerRequest<NID = u64> {
    pub node_id: NID,
    pub api_addr: String,
    pub raft_addr: String,
}

#[since(version = "0.10.1", change = "carry no-op and committed log IDs separately")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct LinearizerData<C>
where C: RaftTypeConfig
{
    pub node_id: C::NodeId,
    pub noop_log_id: LogIdOf<C>,
    pub committed: Option<LogIdOf<C>>,
    pub applied: Option<LogIdOf<C>>,
}

impl<C, SM, D> App<C, SM, D>
where
    C: RaftTypeConfig<Node = NodeInfo>,
    SM: RaftStateMachine<C>,
{
    pub async fn write(self: Arc<Self>, req: C::D) -> Result<ClientWriteResponse<C>, ClientWriteError<C>> {
        self.raft.client_write(req).await.decompose().unwrap()
    }

    pub async fn add_learner(
        self: Arc<Self>,
        req: AddLearnerRequest<C::NodeId>,
    ) -> Result<ClientWriteResponse<C>, ClientWriteError<C>>
    where
        MembershipEditOf<C>: RaftMembershipEditOps<C::Membership>,
    {
        let node = NodeInfo::new(req.raft_addr, req.api_addr);
        self.raft.add_learner(req.node_id, node, true).await.decompose().unwrap()
    }

    pub async fn change_membership(
        self: Arc<Self>,
        node_ids: BTreeSet<C::NodeId>,
    ) -> Result<ClientWriteResponse<C>, ClientWriteError<C>>
    where
        MembershipEditOf<C>: From<ChangeMembers<C::NodeId, C::Node>>,
    {
        let edit = ChangeMembers::<C::NodeId, C::Node>::ReplaceAllVoters(node_ids);
        self.raft.change_membership(edit, false).await.decompose().unwrap()
    }

    pub async fn init(self: Arc<Self>, req: Vec<(C::NodeId, NodeInfo)>) -> Result<(), InitializeError<C>>
    where C::Membership: From<BTreeMap<C::NodeId, NodeInfo>> {
        let mut nodes = BTreeMap::new();
        if req.is_empty() {
            nodes.insert(
                self.id.clone(),
                NodeInfo::new(self.raft_addr.clone(), self.api_addr.clone()),
            );
        } else {
            for (id, node) in req {
                nodes.insert(id, node);
            }
        }

        self.raft.initialize(nodes).await.decompose().unwrap()
    }

    pub async fn metrics(self: Arc<Self>) -> Result<RaftMetrics<C>, Infallible> {
        Ok(self.raft.metrics().borrow_watched().clone())
    }

    /// Report what this node knows about its own state; see [`Readiness`].
    #[since(version = "0.10.2")]
    pub async fn readiness(self: Arc<Self>) -> Result<Readiness<C>, Infallible> {
        let metrics = self.raft.metrics().borrow_watched().clone();

        // A vote names a leader only once a quorum has granted it. The leader's first entry is its
        // no-op, so any applied entry of that leadership means the no-op is applied.
        let leadership = metrics.vote.is_committed().then(|| metrics.vote.leader_id().to_committed());
        let applied_leader_noop = match (&leadership, &metrics.last_applied) {
            (Some(leadership), Some(applied)) => applied.committed_leader_id() == leadership,
            _ => false,
        };

        Ok(Readiness {
            is_leader: metrics.current_leader.as_ref() == Some(&self.id),
            leader: metrics.current_leader,
            applied_leader_noop,
            last_applied: metrics.last_applied,
        })
    }

    pub async fn get_linearizer(self: Arc<Self>) -> Result<LinearizerData<C>, LinearizableReadError<C>> {
        let linearizer = self.raft.get_read_linearizer(ReadPolicy::ReadIndex).await.decompose().unwrap()?;

        Ok(LinearizerData {
            node_id: linearizer.node_id().clone(),
            noop_log_id: linearizer.noop_log_id().clone(),
            committed: linearizer.committed().cloned(),
            applied: linearizer.applied().cloned(),
        })
    }

    pub async fn ensure_linearizable(&self) -> Result<(), LinearizableReadError<C>> {
        self.raft.ensure_linearizable(ReadPolicy::ReadIndex).await.decompose().unwrap()?;
        Ok(())
    }

    /// Wait until this node may serve a linearizable read, or until `timeout` expires.
    ///
    /// Each call asks the leader to authorize this read with [`ReadPolicy::ReadIndex`], so the
    /// authorization comes after the read arrived. Neither an earlier read's authorization nor a
    /// leader role read from metrics can stand in for it: a leader cut off from its quorum keeps
    /// its role. A node that no longer leads names its successor, and the request follows up to
    /// three such redirects. The leader answers without waiting for its own state machine; this
    /// node then waits until its own state machine applies the authorized log ID. Asking the
    /// leader and waiting here share `timeout`.
    ///
    /// Once this returns `Ok`, a read from the local state machine observes every write that
    /// completed before this call began.
    #[since(
        version = "0.10.2",
        change = "take a timeout, follow redirects, and return the authorization"
    )]
    pub async fn ensure_follower_read_ready(&self, timeout: Duration) -> Result<LinearizeState<C>, FollowerReadError<C>>
    where
        LinearizerData<C>: DeserializeOwned,
        LinearizableReadError<C>: DeserializeOwned,
    {
        let deadline = Instant::now() + timeout;

        let (leader_id, leader_addr) = {
            let metrics = self.raft.metrics().borrow_watched().clone();
            let Some(leader_id) = metrics.current_leader else {
                return Err(FollowerReadError::NoLeader);
            };
            let Some(leader_node) = metrics.membership_config.get_node(&leader_id) else {
                return Err(FollowerReadError::NoLeader);
            };
            (leader_id, leader_node.data.clone())
        };

        let client = Client::new(leader_id, leader_addr);
        let linearizer_data = match client.get_linearizer_until(deadline).await {
            Ok(Ok(linearizer_data)) => linearizer_data,
            Ok(Err(refused)) => return Err(FollowerReadError::Refused(refused)),
            Err(e) => {
                let leader_id = client.leader.lock().unwrap().0.clone();
                let reason = e.to_string();
                return Err(FollowerReadError::NoAnswer { leader_id, reason });
            }
        };

        // `applied` is the leader's progress. Paired with the leader's node ID, it lets the wait
        // below skip only on the leader itself; any other node waits for its own state machine.
        let linearizer = Linearizer::new(
            linearizer_data.node_id,
            linearizer_data.noop_log_id,
            linearizer_data.committed,
            linearizer_data.applied,
        );

        let remaining = deadline.saturating_duration_since(Instant::now());
        match linearizer.try_await_ready(&self.raft, Some(remaining)).await {
            Ok(Ok(state)) => Ok(state),
            Ok(Err(state)) => Err(FollowerReadError::NotApplied {
                read_log_id: state.read_log_id().log_id().clone(),
                applied: state.applied().cloned(),
            }),
            Err(fatal) => Err(FollowerReadError::Fatal(fatal)),
        }
    }
}
