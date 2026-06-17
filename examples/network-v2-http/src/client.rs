use std::fmt::Display;
use std::future::Future;
use std::io::Cursor;

use openraft::NodeInfo;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;
use reqwest::Client as HttpClient;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub struct NetworkFactory {
    client: HttpClient,
}

impl Default for NetworkFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkFactory {
    pub fn new() -> Self {
        Self {
            client: HttpClient::builder().no_proxy().build().unwrap(),
        }
    }
}

impl<C> RaftNetworkFactory<C> for NetworkFactory
where C: RaftTypeConfig<Node = NodeInfo>
{
    type Network = Client;

    async fn new_client(&mut self, _target: C::NodeId, node: &NodeInfo) -> Self::Network {
        Client {
            addr: node.raft_addr.clone(),
            client: self.client.clone(),
        }
    }
}

pub struct Client {
    addr: String,
    client: HttpClient,
}

impl Client {
    async fn request<C, Req, Resp>(
        &mut self,
        uri: impl Display,
        req: Req,
        option: &RPCOption,
    ) -> Result<Resp, RPCError<C>>
    where
        C: RaftTypeConfig,
        Req: Serialize,
        Result<Resp, RaftError<C>>: DeserializeOwned,
    {
        let url = format!("http://{}/{}", self.addr, uri);

        let resp = self.client.post(url.clone()).json(&req).timeout(option.soft_ttl()).send().await.map_err(|e| {
            if e.is_connect() {
                RPCError::Unreachable(Unreachable::new(&e))
            } else {
                RPCError::Network(NetworkError::new(&e))
            }
        })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(RPCError::Network(NetworkError::from_string(format!(
                "HTTP {status} from {url}"
            ))));
        }

        let res: Result<Resp, RaftError<C>> = resp.json().await.map_err(|e| NetworkError::new(&e))?;
        res.map_err(|e| RPCError::Unreachable(Unreachable::new(&e)))
    }
}

impl<C> RaftNetworkV2<C> for Client
where C: RaftTypeConfig<Node = NodeInfo>
{
    type SnapshotData = Cursor<Vec<u8>>;

    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C>, RPCError<C>> {
        self.request("append", req, &option).await
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        let req = (vote, snapshot.meta, snapshot.snapshot.into_inner());
        tokio::pin!(cancel);

        tokio::select! {
            closed = &mut cancel => Err(StreamingError::Closed(closed)),
            res = self.request("snapshot", req, &option) => Ok(res?),
        }
    }

    async fn transfer_leader(
        &mut self,
        req: TransferLeaderRequest<C>,
        option: RPCOption,
    ) -> Result<TransferLeaderResponse<C>, RPCError<C>> {
        self.request("transfer-leader", req, &option).await
    }

    async fn vote(&mut self, req: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        self.request("vote", req, &option).await
    }
}
