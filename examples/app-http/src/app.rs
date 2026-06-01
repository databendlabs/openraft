use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;

use openraft::NodeInfo;
use openraft::Raft;
use openraft::RaftMetrics;
use openraft::RaftTypeConfig;
use openraft::ReadPolicy;
use openraft::alias::LogIdOf;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ClientWriteError;
use openraft::errors::Infallible;
use openraft::errors::InitializeError;
use openraft::errors::LinearizableReadError;
use openraft::errors::decompose::DecomposeResult;
use openraft::raft::ClientWriteResponse;
use openraft::raft::linearizable_read::Linearizer;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Client;
use crate::FollowerReadError;

pub struct App<C, SM, D>
where C: RaftTypeConfig<Node = NodeInfo>
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "C::NodeId: Serialize, LogIdOf<C>: Serialize",
    deserialize = "C::NodeId: Deserialize<'de>, LogIdOf<C>: Deserialize<'de>"
))]
pub struct LinearizerData<C>
where C: RaftTypeConfig
{
    pub node_id: C::NodeId,
    pub read_log_id: LogIdOf<C>,
    pub applied: Option<LogIdOf<C>>,
}

impl<C, SM, D> App<C, SM, D>
where C: RaftTypeConfig<Node = NodeInfo>
{
    pub async fn write(self: Arc<Self>, req: C::D) -> Result<ClientWriteResponse<C>, ClientWriteError<C>> {
        self.raft.client_write(req).await.decompose().unwrap()
    }

    pub async fn add_learner(
        self: Arc<Self>,
        req: AddLearnerRequest<C::NodeId>,
    ) -> Result<ClientWriteResponse<C>, ClientWriteError<C>> {
        let node = NodeInfo::new(req.raft_addr, req.api_addr);
        self.raft.add_learner(req.node_id, node, true).await.decompose().unwrap()
    }

    pub async fn change_membership(
        self: Arc<Self>,
        node_ids: BTreeSet<C::NodeId>,
    ) -> Result<ClientWriteResponse<C>, ClientWriteError<C>> {
        self.raft.change_membership(node_ids, false).await.decompose().unwrap()
    }

    pub async fn init(self: Arc<Self>, req: Vec<(C::NodeId, NodeInfo)>) -> Result<(), InitializeError<C>> {
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

    pub async fn get_linearizer(self: Arc<Self>) -> Result<LinearizerData<C>, LinearizableReadError<C>> {
        let linearizer = self.raft.get_read_linearizer(ReadPolicy::ReadIndex).await.decompose().unwrap()?;

        Ok(LinearizerData {
            node_id: linearizer.node_id().clone(),
            read_log_id: linearizer.read_log_id().clone(),
            applied: linearizer.applied().cloned(),
        })
    }

    pub async fn ensure_linearizable(&self) -> Result<(), LinearizableReadError<C>> {
        self.raft.ensure_linearizable(ReadPolicy::ReadIndex).await.decompose().unwrap()?;
        Ok(())
    }

    pub async fn ensure_follower_read_ready(&self) -> Result<(), FollowerReadError>
    where
        LinearizerData<C>: DeserializeOwned,
        LinearizableReadError<C>: DeserializeOwned + Debug,
    {
        let Some(leader_id) = self.raft.current_leader().await else {
            return Err(FollowerReadError {
                message: "No leader available".to_string(),
            });
        };

        let metrics = self.raft.metrics().borrow_watched().clone();
        let Some(leader_node) = metrics.membership_config.membership().get_node(&leader_id) else {
            return Err(FollowerReadError {
                message: format!("Leader node {} not found in membership", leader_id),
            });
        };

        let client = Client::new(leader_id, leader_node.data.clone());
        let linearizer_data = client.get_linearizer().await.map_err(|e| FollowerReadError {
            message: format!("Failed to get linearizer from leader: {}", e),
        })?;

        let linearizer_data = linearizer_data.map_err(|e| FollowerReadError {
            message: format!("Leader returned error: {:?}", e),
        })?;

        let linearizer = Linearizer::new(
            linearizer_data.node_id,
            linearizer_data.read_log_id,
            linearizer_data.applied,
        );

        linearizer.await_ready(&self.raft).await.map_err(|e| FollowerReadError {
            message: format!("Failed to wait for state machine: {:?}", e),
        })?;

        Ok(())
    }
}
