//! HTTP network layer for EzRaft
//!
//! This module provides the built-in HTTP networking that connects Raft nodes.
//! Users don't need to implement anything - the framework handles all RPC communication.
//!
//! # Snapshot transfer
//!
//! All RPCs, snapshot included, are plain request/response JSON over HTTP: a snapshot is
//! small enough at this crate's scale to travel as one POST, which spares the protocol any
//! chunking, resume, or transfer state - a failed transfer is simply sent again.
//!
//! The accepted cost: snapshot bytes ride in JSON, where they serialize as an array of
//! numbers (roughly 4x on the wire). If snapshots outgrow that, `full_snapshot` is the place
//! to switch to e.g. a raw-body streaming POST.

use std::fmt::Display;
use std::future::Future;
use std::io;

use openraft::AnyError;
use openraft::BasicNode;
use openraft::OptionalSend;
use openraft::error::Infallible;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::app::EzApp;
use crate::snapshot::EzSnapshotData;
use crate::snapshot::EzSnapshotMeta;
use crate::type_config::EzVote;
use crate::type_config::OpenRaftTypes;

/// Type alias for OpenRaft types
type C<T> = OpenRaftTypes<T>;

/// HTTP network factory
///
/// Creates HTTP clients to communicate with other Raft nodes.
/// Implements OpenRaft's `RaftNetworkFactory` trait.
pub struct EzNetworkFactory {
    client: Client,
}

impl EzNetworkFactory {
    /// Create a new network factory
    ///
    /// The HTTP client is built once here and handed to every peer, so the whole node shares one
    /// connection pool instead of opening a fresh one per peer.
    pub fn new() -> Result<Self, io::Error> {
        let client = Client::builder().no_proxy().build().map_err(io::Error::other)?;

        Ok(Self { client })
    }
}

impl<T> RaftNetworkFactory<C<T>> for EzNetworkFactory
where T: EzApp
{
    type Network = Network;

    async fn new_client(&mut self, _target: u64, node: &BasicNode) -> Self::Network {
        let addr = node.addr.clone();
        let client = self.client.clone();

        Network { addr, client }
    }
}

/// HTTP network client for a single Raft node
pub struct Network {
    addr: String,
    client: Client,
}

impl Network {
    /// Send an HTTP POST request to a target node
    ///
    /// The request is given the timeout budget openraft computed for it: it is dropped at
    /// `soft_ttl` so this side reports the failure before openraft abandons the call itself.
    async fn request<Req, Resp, Err, Cfg>(
        &mut self,
        uri: impl Display,
        req: Req,
        option: &RPCOption,
    ) -> Result<Result<Resp, Err>, RPCError<Cfg>>
    where
        Cfg: openraft::RaftTypeConfig,
        Req: Serialize + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned,
    {
        let url = format!("http://{}/{}", self.addr, uri);

        let resp = self.client.post(url.clone()).timeout(option.soft_ttl()).json(&req).send().await.map_err(|e| {
            // A peer that does not answer in time is treated like one that cannot be reached, so
            // replication backs off instead of hammering it.
            if e.is_connect() || e.is_timeout() {
                RPCError::Unreachable(Unreachable::new(&e))
            } else {
                RPCError::Network(NetworkError::new(&e))
            }
        })?;

        // The body is a serialized `Result` only on success; any other status carries a
        // plain-text reason that would otherwise surface as an opaque deserialization failure.
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let err = AnyError::error(format!("{} responded {}: {}", url, status, body));
            return Err(RPCError::Network(NetworkError::new(&err)));
        }

        let res: Result<Resp, Err> = resp.json().await.map_err(|e| NetworkError::new(&e))?;

        Ok(res)
    }
}

/// Implement RaftNetworkV2 for HTTP transport
impl<T> RaftNetworkV2<C<T>> for Network
where T: EzApp
{
    type SnapshotData = EzSnapshotData;

    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<C<T>>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<C<T>>, RPCError<C<T>>> {
        let res = self.request::<_, _, Infallible, C<T>>("raft/append", req, &option).await?;
        Ok(res.unwrap())
    }

    /// Send the whole snapshot in one request; the receiving side hands it to
    /// [`Raft::install_full_snapshot`](openraft::Raft::install_full_snapshot).
    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C<T>>,
        snapshot: SnapshotOf<C<T>, Self::SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C<T>>, StreamingError<C<T>>> {
        let req = SnapshotTransfer {
            vote,
            meta: snapshot.meta,
            data: snapshot.snapshot.into_inner(),
        };
        tokio::pin!(cancel);

        tokio::select! {
            closed = &mut cancel => Err(StreamingError::Closed(closed)),
            res = self.request::<_, _, Infallible, C<T>>("raft/snapshot", req, &option) => Ok(res?.unwrap()),
        }
    }

    async fn vote(&mut self, req: VoteRequest<C<T>>, option: RPCOption) -> Result<VoteResponse<C<T>>, RPCError<C<T>>> {
        let res = self.request::<_, _, Infallible, C<T>>("raft/vote", req, &option).await?;
        Ok(res.unwrap())
    }
}

/// Wire format of the one-shot snapshot POST to `/raft/snapshot`
///
/// Carries exactly what [`Raft::install_full_snapshot`](openraft::Raft::install_full_snapshot)
/// needs on the receiving side.
#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct SnapshotTransfer {
    pub vote: EzVote,
    pub meta: EzSnapshotMeta,
    pub data: Vec<u8>,
}
