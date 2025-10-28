use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::error::CheckIsLeaderError;
use openraft::error::ClientWriteError;
use openraft::error::ForwardToLeader;
use openraft::error::Infallible;
use openraft::error::InitializeError;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::Unreachable;
use openraft::raft::ClientWriteResponse;
use openraft::BasicNode;
use openraft::RaftMetrics;
use openraft::RaftTypeConfig;
use openraft::TryAsRef;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct FollowerReadError {
    pub message: String,
}

impl fmt::Display for FollowerReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FollowerReadError {}

pub struct ExampleClient<C>
where C: RaftTypeConfig
{
    /// The leader node to send request to.
    ///
    /// All traffic should be sent to the leader in a cluster.
    pub leader: Arc<Mutex<(C::NodeId, String)>>,

    pub inner: Client,
}

impl<C> ExampleClient<C>
where C: RaftTypeConfig<Node = BasicNode>
{
    /// Create a client with a leader node id and a node manager to get node address by node id.
    pub fn new(leader_id: C::NodeId, leader_addr: String) -> Self {
        Self {
            leader: Arc::new(Mutex::new((leader_id, leader_addr))),
            inner: Client::builder().no_proxy().build().unwrap(),
        }
    }

    // --- Application API

    /// Submit a write request to the raft cluster.
    ///
    /// The request will be processed by raft protocol: it will be replicated to a quorum and then
    /// will be applied to state machine.
    ///
    /// The result of applying the request will be returned.
    pub async fn write(&self, req: &C::D) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, RPCError<C>> {
        self.send_with_forwarding("write", Some(req), 3).await
    }

    /// Read value by key, in an inconsistent mode.
    ///
    /// This method may return stale value because it does not force to read on a legal leader.
    pub async fn read(&self, req: &String) -> Result<String, RPCError<C>> {
        let res = self.send::<_, _, Infallible>("read", Some(req)).await?;
        Ok(res.unwrap())
    }

    /// Consistent Read value by key, in an inconsistent mode.
    ///
    /// This method MUST return consistent value or CheckIsLeaderError.
    pub async fn linearizable_read(&self, req: &String) -> Result<Result<String, CheckIsLeaderError<C>>, RPCError<C>> {
        self.send_with_forwarding("linearizable_read", Some(req), 0).await
    }

    /// Same as linearizable_read() but will automatically forward the request to the current leader
    /// if the node is not a leader.
    pub async fn linearizable_read_auto_forward(
        &self,
        req: &String,
    ) -> Result<Result<String, CheckIsLeaderError<C>>, RPCError<C>> {
        self.send_with_forwarding("linearizable_read", Some(req), 3).await
    }

    /// Perform a linearizable read on a follower.
    ///
    /// This method demonstrates follower reads: it fetches a linearizer from the leader,
    /// waits for the local state machine to catch up, then reads from the local state machine.
    ///
    /// Returns the value if successful, or an error from the follower.
    pub async fn follower_read(&self, req: &String) -> Result<Result<String, FollowerReadError>, RPCError<C>> {
        self.send("follower_read", Some(req)).await
    }

    // --- Cluster management API

    /// Initialize a cluster of only the node that receives this request.
    ///
    /// This is the first step to initialize a cluster.
    /// With a initialized cluster, new node can be added with [`write`].
    /// Then setup replication with [`add_learner`].
    /// Then make the new node a member with [`change_membership`].
    pub async fn init(&self) -> Result<Result<(), InitializeError<C>>, RPCError<C>> {
        self.send("init", Some::<&[(); 0]>(&[])).await
    }

    /// Add a node as learner.
    ///
    /// The node to add has to exist, i.e., being added with `write(ExampleRequest::AddNode{})`
    pub async fn add_learner(
        &self,
        req: (C::NodeId, String),
    ) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, RPCError<C>> {
        self.send_with_forwarding("add-learner", Some(&req), 0).await
    }

    /// Change membership to the specified set of nodes.
    ///
    /// All nodes in `req` have to be already added as learner with [`add_learner`],
    /// or an error [`LearnerNotFound`] will be returned.
    pub async fn change_membership(
        &self,
        req: &BTreeSet<C::NodeId>,
    ) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, RPCError<C>> {
        self.send_with_forwarding("change-membership", Some(req), 0).await
    }

    /// Get the metrics about the cluster.
    ///
    /// Metrics contains various information about the cluster, such as current leader,
    /// membership config, replication status etc.
    /// See [`RaftMetrics`].
    pub async fn metrics(&self) -> Result<RaftMetrics<C>, RPCError<C>> {
        let res = self.send::<_, _, Infallible>("metrics", None::<&()>).await?;
        Ok(res.unwrap())
    }

    // --- Internal methods

    /// Send RPC to leader node without retry.
    ///
    /// It sends out a POST request if `req` is Some. Otherwise a GET request.
    /// The remote endpoint must respond a reply in form of `Result<Resp, Err>`.
    async fn send<Req, Resp, Err>(&self, uri: &str, req: Option<&Req>) -> Result<Result<Resp, Err>, RPCError<C>>
    where
        Req: Serialize + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned,
    {
        let (_leader_id, url) = {
            let t = self.leader.lock().unwrap();
            let target_addr = &t.1;
            (t.0.clone(), format!("http://{}/{}", target_addr, uri))
        };

        let resp = if let Some(r) = req {
            println!(
                ">>> client send request to {}: {}",
                url,
                serde_json::to_string_pretty(&r).unwrap()
            );
            self.inner.post(url.clone()).json(r)
        } else {
            println!(">>> client send request to {}", url,);
            self.inner.get(url.clone())
        }
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                // `Unreachable` informs the caller to backoff for a short while to avoid error log flush.
                RPCError::Unreachable(Unreachable::new(&e))
            } else {
                RPCError::Network(NetworkError::new(&e))
            }
        })?;

        let res: Result<Resp, Err> = resp.json().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        println!(
            "<<< client recv reply from {}: {}",
            url,
            serde_json::to_string_pretty(&res).unwrap()
        );

        Ok(res)
    }

    /// Try the best to send a request to the leader.
    ///
    /// If the target node is not a leader, a `ForwardToLeader` error will be
    /// returned and this client will retry at most `retry` times to contact the updated leader.
    ///
    /// `retry==0` means no retry is made after the first failure.
    async fn send_with_forwarding<Req, Resp, Err>(
        &self,
        uri: &str,
        req: Option<&Req>,
        mut retry: usize,
    ) -> Result<Result<Resp, Err>, RPCError<C>>
    where
        Req: Serialize + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned + TryAsRef<ForwardToLeader<C>> + Clone,
    {
        loop {
            let res: Result<Resp, Err> = self.send(uri, req).await?;

            let rpc_err = match res {
                Ok(x) => return Ok(Ok(x)),
                Err(rpc_err) => rpc_err,
            };

            if let Some(ForwardToLeader {
                leader_id: Some(leader_id),
                leader_node: Some(leader_node),
                ..
            }) = rpc_err.try_as_ref()
            {
                // Update target to the new leader.
                {
                    let mut t = self.leader.lock().unwrap();
                    let api_addr = leader_node.addr.clone();
                    *t = (leader_id.clone(), api_addr);
                }

                if retry > 0 {
                    retry -= 1;
                    continue;
                }
            }

            return Ok(Err(rpc_err));
        }
    }
}
