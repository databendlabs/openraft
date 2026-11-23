use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use openraft::NodeInfo;
use openraft::RaftMetrics;
use openraft::RaftTypeConfig;
use openraft::TryAsRef;
use openraft::alias::LogIdOf;
use openraft::errors::ClientWriteError;
use openraft::errors::Fatal;
use openraft::errors::ForwardToLeader;
use openraft::errors::Infallible;
use openraft::errors::InitializeError;
use openraft::errors::LinearizableReadError;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::errors::Unreachable;
use openraft::macros::since;
use openraft::raft::ClientWriteResponse;
use reqwest::Client as HttpClient;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::time::Instant;

use crate::LinearizerData;
use crate::Readiness;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// How long [`Client::write_with_retries`] waits for the response to one attempt.
const WRITE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);

/// How long [`Client::write_with_retries`] waits before it resends a write.
const WRITE_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Why a node did not serve a follower read.
#[since(version = "0.10.2", change = "name the reason a follower read failed")]
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(bound = "")]
pub enum FollowerReadError<C>
where C: RaftTypeConfig
{
    /// This node knows of no leader, or not the address of the leader it knows of.
    NoLeader,

    /// No answer arrived from `leader_id`, the last leader asked to authorize the read: the request
    /// failed, or the read's deadline expired first.
    NoAnswer { leader_id: C::NodeId, reason: String },

    /// The leader did not authorize the read, for example because it could not confirm its
    /// leadership with a quorum, or because the redirects to a newer leader ran out.
    Refused(LinearizableReadError<C>),

    /// The read's deadline expired before this node applied `read_log_id`, the log ID the leader
    /// authorized the read at. `applied` is the last log ID this node had applied.
    NotApplied {
        read_log_id: LogIdOf<C>,
        applied: Option<LogIdOf<C>>,
    },

    /// Raft stopped on this node while the read waited for it.
    Fatal(Fatal<C>),
}

impl<C> fmt::Display for FollowerReadError<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLeader => write!(f, "no known leader to authorize the read"),
            Self::NoAnswer { leader_id, reason } => write!(f, "no answer from leader {leader_id}: {reason}"),
            Self::Refused(e) => write!(f, "the leader did not authorize the read: {e}"),
            Self::NotApplied {
                read_log_id,
                applied: Some(applied),
            } => write!(f, "deadline expired before applying {read_log_id}; applied {applied}"),
            Self::NotApplied {
                read_log_id,
                applied: None,
            } => write!(f, "deadline expired before applying {read_log_id}; applied nothing"),
            Self::Fatal(e) => write!(f, "Raft stopped: {e}"),
        }
    }
}

impl<C> std::error::Error for FollowerReadError<C> where C: RaftTypeConfig {}

pub struct Client<C>
where C: RaftTypeConfig
{
    /// The leader node to send request to.
    ///
    /// All traffic should be sent to the leader in a cluster.
    pub leader: Arc<Mutex<(C::NodeId, String)>>,

    pub inner: HttpClient,
}

impl<C> Client<C>
where C: RaftTypeConfig<Node = NodeInfo>
{
    /// Create a client with a leader node id and a node manager to get node address by node id.
    pub fn new(leader_id: C::NodeId, leader_addr: String) -> Self {
        Self {
            leader: Arc::new(Mutex::new((leader_id, leader_addr))),
            inner: HttpClient::builder().no_proxy().build().unwrap(),
        }
    }

    // --- Application API

    /// Submit a write request to the raft cluster.
    ///
    /// The request will be processed by raft protocol: it will be replicated to a quorum and then
    /// will be applied to state machine.
    ///
    /// The result of applying the request will be returned.
    ///
    /// A node that answers [`ClientWriteError::ForwardToLeader`] did not append the write, so the
    /// write is resent to the leader it names, at most three times. A node that answers
    /// [`ClientWriteError::LogEntryDiscarded`] appended the write and then dropped it, but the
    /// write may still commit through another leader, and a resent copy would then apply twice.
    /// That error is returned without resending, and the next request goes to the leader it
    /// names. To resend a write whose outcome is unknown, see [`Self::write_with_retries`].
    #[since(version = "0.10.2", change = "return LogEntryDiscarded without resending the write")]
    pub async fn write(&self, req: &C::D) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, RPCError<C>> {
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let mut redirects = 3;

        loop {
            let res = self.send_until("write", Some(req), deadline).await?;

            let redirected = match &res {
                Err(ClientWriteError::ForwardToLeader(forward)) => self.follow_leader_hint(forward),
                Err(ClientWriteError::LogEntryDiscarded(forward)) => {
                    self.follow_leader_hint(forward);
                    false
                }
                _ => false,
            };

            if !redirected || redirects == 0 {
                return Ok(res);
            }
            redirects -= 1;
        }
    }

    /// Write `req`, and resend it until its outcome is known or the ten-second operation budget
    /// runs out.
    ///
    /// A write whose response fails or does not arrive within two seconds, or that ends with
    /// [`ClientWriteError::LogEntryDiscarded`], may have committed, so a resent copy may apply it
    /// twice. Use this method only for a request that the state machine applies at most once per
    /// request ID, such as `types_kv::DedupRequest`: a resent copy gets the response of the first
    /// application. Each attempt goes to the leader named by the error of the previous one. A
    /// write refused for any reason other than [`ClientWriteError::ForwardToLeader`] is returned
    /// at once.
    ///
    /// When the budget runs out, the last result is returned and the outcome is still unknown.
    /// Resend the same request to learn it.
    #[since(version = "0.10.2")]
    pub async fn write_with_retries(
        &self,
        req: &C::D,
    ) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, RPCError<C>> {
        let deadline = Instant::now() + OPERATION_TIMEOUT;

        loop {
            let attempt_deadline = deadline.min(Instant::now() + WRITE_ATTEMPT_TIMEOUT);
            let res = self.send_until("write", Some(req), attempt_deadline).await;

            match &res {
                Ok(Ok(_)) => return res,
                Ok(Err(ClientWriteError::ForwardToLeader(forward) | ClientWriteError::LogEntryDiscarded(forward))) => {
                    self.follow_leader_hint(forward);
                }
                Ok(Err(_)) => return res,
                // No response arrived: the write may have committed.
                Err(_) => {}
            }

            if Instant::now() + WRITE_RETRY_DELAY >= deadline {
                return res;
            }
            tokio::time::sleep(WRITE_RETRY_DELAY).await;
        }
    }

    /// Read value by key, in an inconsistent mode.
    ///
    /// This method may return stale value because it does not force to read on a legal leader.
    pub async fn read(&self, req: &String) -> Result<C::R, RPCError<C>> {
        let res = self.send::<_, _, Infallible>("read", Some(req)).await?;
        Ok(res.unwrap())
    }

    /// Consistent Read value by key, in an inconsistent mode.
    ///
    /// This method MUST return consistent value or LinearizableReadError.
    pub async fn linearizable_read(&self, req: &String) -> Result<Result<C::R, LinearizableReadError<C>>, RPCError<C>> {
        self.send_with_forwarding("linearizable_read", Some(req), 0).await
    }

    /// Same as linearizable_read() but will automatically forward the request to the current leader
    /// if the node is not a leader.
    pub async fn linearizable_read_auto_forward(
        &self,
        req: &String,
    ) -> Result<Result<C::R, LinearizableReadError<C>>, RPCError<C>> {
        self.send_with_forwarding("linearizable_read", Some(req), 3).await
    }

    /// Perform a linearizable read on a follower.
    ///
    /// This method demonstrates follower reads: the target node asks the leader to authorize the
    /// read, waits until its own state machine applies the authorized log ID, then reads from its
    /// local state machine.
    ///
    /// Returns the value if successful, or an error from the follower.
    pub async fn follower_read(&self, req: &String) -> Result<Result<C::R, FollowerReadError<C>>, RPCError<C>> {
        self.send("follower_read", Some(req)).await
    }

    pub async fn get_linearizer(&self) -> Result<Result<LinearizerData<C>, LinearizableReadError<C>>, RPCError<C>> {
        self.send("get_linearizer", None::<&()>).await
    }

    /// Ask the leader to authorize a linearizable read, following up to three redirects to a newer
    /// leader, and give up at `deadline`.
    #[since(version = "0.10.2")]
    pub async fn get_linearizer_until(
        &self,
        deadline: Instant,
    ) -> Result<Result<LinearizerData<C>, LinearizableReadError<C>>, RPCError<C>> {
        self.send_with_forwarding_until("get_linearizer", None::<&()>, 3, deadline).await
    }

    /// Get what the node reports about its own state; see [`Readiness`].
    #[since(version = "0.10.2")]
    pub async fn readiness(&self) -> Result<Readiness<C>, RPCError<C>> {
        let res = self.send::<_, _, Infallible>("readiness", None::<&()>).await?;
        Ok(res.unwrap())
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
    pub async fn add_learner<Req>(
        &self,
        req: &Req,
    ) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, RPCError<C>>
    where
        Req: Serialize + fmt::Debug + 'static,
    {
        self.send_with_forwarding("add-learner", Some(req), 0).await
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
        Req: Serialize + fmt::Debug + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned,
    {
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        self.send_until(uri, req, deadline).await
    }

    async fn send_until<Req, Resp, Err>(
        &self,
        uri: &str,
        req: Option<&Req>,
        deadline: Instant,
    ) -> Result<Result<Resp, Err>, RPCError<C>>
    where
        Req: Serialize + fmt::Debug + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned,
    {
        let operation = async {
            log::debug!("app-http client: start, send to {}; request: {:?}", uri, req);

            let (_leader_id, url) = {
                let t = self.leader.lock().unwrap();
                let target_addr = &t.1;
                (t.0.clone(), format!("http://{}/{}", target_addr, uri))
            };

            let resp = if let Some(r) = req {
                let body = serde_json::to_vec(r).map_err(|error| RPCError::Network(NetworkError::new(&error)))?;
                self.inner.post(url.clone()).header(reqwest::header::CONTENT_TYPE, "application/json").body(body)
            } else {
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

            let resp = resp.error_for_status().map_err(|error| RPCError::Network(NetworkError::new(&error)))?;
            let res: Result<Resp, Err> = resp.json().await.map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
            Ok(res)
        };
        tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|error| RPCError::Network(NetworkError::new(&error)))?
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
        retry: usize,
    ) -> Result<Result<Resp, Err>, RPCError<C>>
    where
        Req: Serialize + fmt::Debug + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned + TryAsRef<ForwardToLeader<C>> + Clone,
    {
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        self.send_with_forwarding_until(uri, req, retry, deadline).await
    }

    async fn send_with_forwarding_until<Req, Resp, Err>(
        &self,
        uri: &str,
        req: Option<&Req>,
        mut retry: usize,
        deadline: Instant,
    ) -> Result<Result<Resp, Err>, RPCError<C>>
    where
        Req: Serialize + fmt::Debug + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned + TryAsRef<ForwardToLeader<C>> + Clone,
    {
        loop {
            let res: Result<Resp, Err> = self.send_until(uri, req, deadline).await?;

            let rpc_err = match res {
                Ok(x) => return Ok(Ok(x)),
                Err(rpc_err) => rpc_err,
            };

            let redirected = rpc_err.try_as_ref().is_some_and(|forward| self.follow_leader_hint(forward));
            if redirected && retry > 0 {
                retry -= 1;
                continue;
            }

            return Ok(Err(rpc_err));
        }
    }

    /// Send later requests to the leader that `forward` names, if it names one with an address.
    ///
    /// Returns whether it named one.
    fn follow_leader_hint(&self, forward: &ForwardToLeader<C>) -> bool {
        let (Some(leader_id), Some(leader_node)) = (&forward.leader_id, &forward.leader_node) else {
            return false;
        };

        let mut target = self.leader.lock().unwrap();
        *target = (leader_id.clone(), leader_node.data.clone());
        true
    }
}

#[cfg(test)]
mod client_test;
