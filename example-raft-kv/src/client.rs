use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use openraft::error::AddLearnerError;
use openraft::error::ClientWriteError;
use openraft::error::ForwardToLeader;
use openraft::error::Infallible;
use openraft::error::InitializeError;
use openraft::error::NetworkError;
use openraft::error::NodeNotFound;
use openraft::error::RPCError;
use openraft::error::RemoteError;
use openraft::raft::AddLearnerResponse;
use openraft::raft::ClientWriteResponse;
use openraft::NodeId;
use openraft::RaftMetrics;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;

use crate::ExampleRequest;
use crate::ExampleResponse;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Empty {}

#[async_trait]
pub trait NodeManager {
    async fn get_node_address(&self, node_id: NodeId) -> Result<String, NodeNotFound>;
}

#[async_trait]
impl<T> NodeManager for T
where T: Fn(NodeId) -> Result<String, NodeNotFound> + Sync
{
    async fn get_node_address(&self, node_id: NodeId) -> Result<String, NodeNotFound> {
        self(node_id)
    }
}

pub struct ExampleClient {
    /// The leader node to send request to.
    ///
    /// All traffic should be sent to the leader in a cluster.
    pub leader_id: Arc<Mutex<NodeId>>,

    /// Raft itself does not store node addresses.
    /// Thus when a `ForwardToLeader` error is returned, the client need to find the node address by itself.
    pub node_manager: Arc<dyn NodeManager>,

    pub inner: Client,
}

impl ExampleClient {
    /// Create a client with a leader node id and a node manager to get node address by node id.
    pub fn new(leader_id: NodeId, node_manager: Arc<dyn NodeManager>) -> Self {
        Self {
            leader_id: Arc::new(Mutex::new(leader_id)),
            inner: reqwest::Client::new(),
            node_manager,
        }
    }

    // --- Application API

    /// Submit a write request to the raft cluster.
    ///
    /// The request will be processed by raft protocol: it will be replicated to a quorum and then will be applied to
    /// state machine.
    ///
    /// The result of applying the request will be returned.
    pub async fn write(
        &self,
        req: &ExampleRequest,
    ) -> Result<ClientWriteResponse<ExampleResponse>, RPCError<ClientWriteError>> {
        self.send_rpc_to_leader("write", Some(req)).await
    }

    /// Read value by key, in an inconsistent mode.
    ///
    /// This method may return stale value because it does not force to read on a legal leader.
    pub async fn read(&self, req: &String) -> Result<String, RPCError<Infallible>> {
        let target = self.get_leader_id();
        self.send_rpc(target, "read", Some(req)).await
    }

    // --- Cluster management API

    /// Initialize a cluster of only the node that receives this request.
    ///
    /// This is the first step to initialize a cluster.
    /// With a initialized cluster, new node can be added with [`write`].
    /// Then setup replication with [`add_learner`].
    /// Then make the new node a member with [`change_membership`].
    pub async fn init(&self) -> Result<(), RPCError<InitializeError>> {
        let target = self.get_leader_id();
        self.send_rpc(target, "init", Some(&Empty {})).await
    }

    /// Add a node as learner.
    ///
    /// The node to add has to exist, i.e., being added with `write(ExampleRequest::AddNode{})`
    pub async fn add_learner(&self, req: &NodeId) -> Result<AddLearnerResponse, RPCError<AddLearnerError>> {
        self.send_rpc_to_leader("add-learner", Some(req)).await
    }

    /// Change membership to the specified set of nodes.
    ///
    /// All nodes in `req` have to be already added as learner with [`add_learner`],
    /// or an error [`LearnerNotFound`] will be returned.
    pub async fn change_membership(
        &self,
        req: &BTreeSet<NodeId>,
    ) -> Result<ClientWriteResponse<ExampleResponse>, RPCError<ClientWriteError>> {
        self.send_rpc_to_leader("change-membership", Some(req)).await
    }

    /// Get the metrics about the cluster.
    pub async fn metrics(&self) -> Result<RaftMetrics, RPCError<Infallible>> {
        let target = self.get_leader_id();
        self.send_rpc(target, "metrics", None::<&()>).await
    }

    /// List all known nodes.
    ///
    /// A known node does not have to be a member or learner.
    /// It is just a node that the cluster can use as learner or a member.
    pub async fn list_nodes(&self) -> Result<BTreeMap<NodeId, String>, RPCError<Infallible>> {
        let target = self.get_leader_id();
        self.send_rpc(target, "list-nodes", None::<&()>).await
    }

    // --- Internal methods

    /// Get the last known leader node id by this client.
    fn get_leader_id(&self) -> NodeId {
        let t = self.leader_id.lock().unwrap();
        *t
    }

    /// Send RPC to specified node.
    ///
    /// It sends out a POST request if `req` is Some. Otherwise a GET request.
    /// The remote endpoint must respond a reply in form of `Result<T, E>`.
    /// An `Err` happened on remote will be wrapped in an [`RPCError::RemoteError`].
    async fn send_rpc<Req, Resp, Err>(
        &self,
        target: NodeId,
        uri: &str,
        req: Option<&Req>,
    ) -> Result<Resp, RPCError<Err>>
    where
        Req: Serialize + 'static,
        Resp: DeserializeOwned,
        Err: std::error::Error + DeserializeOwned,
    {
        let addr = self.node_manager.get_node_address(target).await?;

        let url = format!("http://{}/{}", addr, uri);

        let resp = if let Some(r) = req {
            self.inner.post(url).json(r)
        } else {
            self.inner.get(url)
        }
        .send()
        .await
        .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        let res: Result<Resp, Err> = resp.json().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        res.map_err(|e| RPCError::RemoteError(RemoteError::new(target, e)))
    }

    /// Try the best to send a request to the leader.
    ///
    /// If the target node is not a leader, a `ForwardToLeader` error will be
    /// returned and this client will retry at most 3 times to contact the updated leader.
    async fn send_rpc_to_leader<Req, Resp, Err>(&self, uri: &str, req: Option<&Req>) -> Result<Resp, RPCError<Err>>
    where
        Req: Serialize + 'static,
        Resp: DeserializeOwned,
        Err: std::error::Error + DeserializeOwned + TryInto<ForwardToLeader> + Clone,
    {
        // Retry at most 3 times to find a valid leader.
        let mut n_retry = 3;

        loop {
            let target = self.get_leader_id();

            let res: Result<Resp, RPCError<Err>> = self.send_rpc(target, uri, req).await;

            let rpc_err = match res {
                Ok(x) => return Ok(x),
                Err(rpc_err) => rpc_err,
            };

            if let RPCError::RemoteError(remote_err) = &rpc_err {
                let forward_err_res = <Err as TryInto<ForwardToLeader>>::try_into(remote_err.source.clone());

                if let Ok(ForwardToLeader {
                    leader_id: Some(leader_id),
                }) = forward_err_res
                {
                    // Update target to the new leader.
                    {
                        let mut t = self.leader_id.lock().unwrap();
                        *t = leader_id;
                    }

                    n_retry -= 1;
                    if n_retry > 0 {
                        continue;
                    }
                }
            }

            return Err(rpc_err);
        }
    }
}
