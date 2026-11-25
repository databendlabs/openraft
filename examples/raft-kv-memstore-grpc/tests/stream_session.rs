use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use futures::Stream;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::channel::oneshot;
use maplit::btreemap;
use maplit::btreeset;
use openraft::Config;
use openraft::OptionalSend;
use openraft::RaftNetworkFactory;
use openraft::base::BoxFuture;
use openraft::base::BoxStream;
use openraft::network::NetStreamAppend;
use openraft::network::NetVote;
use openraft::network::RPCOption;
use openraft::type_config::TypeConfigExt;
use raft_kv_memstore_grpc::Raft;
use raft_kv_memstore_grpc::TypeConfig;
use raft_kv_memstore_grpc::grpc::raft_service::RaftServiceImpl;
use raft_kv_memstore_grpc::network::NetworkConnection;
use raft_kv_memstore_grpc::protobuf as pb;
use raft_kv_memstore_grpc::protobuf::raft_service_server::RaftService;
use raft_kv_memstore_grpc::protobuf::raft_service_server::RaftServiceServer;
use raft_kv_memstore_grpc::store::LogStore;
use raft_kv_memstore_grpc::store::StateMachineStore;
use raft_kv_memstore_grpc::typ::*;
use tonic::Request;
use tonic::Response;
use tonic::Status;
use tonic::Streaming;

const TIMEOUT: Duration = Duration::from_secs(5);

// 0: dropped, 1: live, 2: live and has sent entries (not a heartbeat).
struct RequestStream {
    state: AtomicUsize,
    open_failed: AtomicBool,
}

struct Tracked<S> {
    input: Option<S>,
    alive: Arc<RequestStream>,
}

impl<S> Stream for Tracked<S>
where S: Stream<Item = AppendEntriesRequest> + Unpin
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let result = Pin::new(self.input.as_mut().unwrap()).poll_next(cx);
        if let Poll::Ready(Some(ref request)) = result
            && !request.entries.is_empty()
        {
            self.alive.state.store(2, Ordering::SeqCst);
        }
        result
    }
}

impl<S> Drop for Tracked<S> {
    fn drop(&mut self) {
        drop(self.input.take());
        self.alive.state.store(0, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct Network {
    streams: Arc<Mutex<Vec<Arc<RequestStream>>>>,
}

struct Connection {
    inner: NetworkConnection,
    streams: Arc<Mutex<Vec<Arc<RequestStream>>>>,
}

impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = Connection;

    async fn new_client(&mut self, _target: u64, node: &pb::Node) -> Connection {
        Connection {
            inner: NetworkConnection::new(node.clone()),
            streams: self.streams.clone(),
        }
    }
}

impl NetStreamAppend<TypeConfig> for Connection {
    fn stream_append<'s, S>(
        &'s mut self,
        input: S,
        option: RPCOption,
    ) -> BoxFuture<'s, Result<BoxStream<'s, Result<AppendResult, RPCError>>, RPCError>>
    where
        S: Stream<Item = AppendEntriesRequest> + OptionalSend + Unpin + 'static,
    {
        let alive = Arc::new(RequestStream {
            state: AtomicUsize::new(1),
            open_failed: AtomicBool::new(false),
        });
        self.streams.lock().unwrap().push(alive.clone());
        Box::pin(async move {
            let result = self
                .inner
                .stream_append(
                    Tracked {
                        input: Some(input),
                        alive: alive.clone(),
                    },
                    option,
                )
                .await;
            alive.open_failed.store(result.is_err(), Ordering::SeqCst);
            result
        })
    }
}

impl openraft::network::NetBackoff<TypeConfig> for Connection {
    fn backoff(&self) -> Option<openraft::network::Backoff> {
        self.inner.backoff()
    }
}

impl NetVote<TypeConfig> for Connection {
    async fn vote(&mut self, req: VoteRequest, option: RPCOption) -> Result<VoteResponse, RPCError> {
        self.inner.vote(req, option).await
    }
}

impl openraft::network::NetSnapshot<TypeConfig> for Connection {
    type SnapshotData = Vec<u8>;

    async fn full_snapshot(
        &mut self,
        vote: Vote,
        snapshot: Snapshot,
        cancel: impl Future<Output = openraft::errors::ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse, StreamingError> {
        self.inner.full_snapshot(vote, snapshot, cancel, option).await
    }
}

impl openraft::network::NetTransferLeader<TypeConfig> for Connection {
    async fn transfer_leader(
        &mut self,
        req: openraft::raft::TransferLeaderRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<openraft::raft::TransferLeaderResponse<TypeConfig>, RPCError> {
        self.inner.transfer_leader(req, option).await
    }
}

// A bidirectional peer may keep reading the upload after closing the response half.
struct Peer {
    inner: RaftServiceImpl,
    raft: Raft,
    close: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    fail_open: Arc<AtomicBool>,
    failed_opens: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl RaftService for Peer {
    async fn vote(&self, r: Request<pb::VoteRequest>) -> Result<Response<pb::VoteResponse>, Status> {
        self.inner.vote(r).await
    }

    async fn pre_vote(&self, r: Request<pb::VoteRequest>) -> Result<Response<pb::VoteResponse>, Status> {
        self.inner.pre_vote(r).await
    }

    async fn append_entries(
        &self,
        r: Request<pb::AppendEntriesRequest>,
    ) -> Result<Response<pb::AppendEntriesResponse>, Status> {
        self.inner.append_entries(r).await
    }

    async fn snapshot(
        &self,
        r: Request<Streaming<pb::SnapshotRequest>>,
    ) -> Result<Response<pb::SnapshotResponse>, Status> {
        self.inner.snapshot(r).await
    }

    type StreamAppendStream = BoxStream<'static, Result<pb::AppendEntriesResponse, Status>>;

    async fn stream_append(
        &self,
        r: Request<Streaming<pb::AppendEntriesRequest>>,
    ) -> Result<Response<Self::StreamAppendStream>, Status> {
        let mut input = r.into_inner();
        let first = input.message().await?.ok_or_else(|| Status::invalid_argument("empty upload"))?;
        // Heartbeats use this RPC too. Fail only a log-carrying replication session.
        if !first.entries.is_empty() && self.fail_open.swap(false, Ordering::SeqCst) {
            self.failed_opens.fetch_add(1, Ordering::SeqCst);
            TypeConfig::spawn(async move { while input.next().await.is_some() {} });
            return Err(Status::unavailable("stream not accepted"));
        }
        let input = futures::stream::once(futures::future::ready(Ok(first))).chain(input);
        let input = input.filter_map(|r| async move { r.ok().map(Into::into) });
        let mut output = Box::pin(self.raft.stream_append(input));
        let (tx, rx) = mpsc::unbounded();
        TypeConfig::spawn(async move {
            while let Some(response) = output.next().await {
                // Continue reading the upload even when the response half has ended.
                let response = response.map(Into::into).map_err(|e| Status::internal(e.to_string()));
                let _ = tx.unbounded_send(response);
            }
        });
        let (close_tx, close_rx) = oneshot::channel();
        self.close.lock().unwrap().push(close_tx);
        let close = async move {
            let _ = close_rx.await;
        };
        Ok(Response::new(Box::pin(rx.take_until(close))))
    }
}

async fn check_session(fail_open: bool) {
    tracing::info!(fail_open, "--- start a two-node cluster over the example tonic client");
    let config = Arc::new(
        Config {
            enable_elect: false,
            ..Config::default()
        }
        .validate()
        .unwrap(),
    );
    let streams = Arc::new(Mutex::new(Vec::new()));
    let network = Network {
        streams: streams.clone(),
    };
    let follower = Raft::new(
        2,
        config.clone(),
        network.clone(),
        LogStore::default(),
        Arc::new(StateMachineStore::default()),
    )
    .await
    .unwrap();
    let address = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let close = Arc::new(Mutex::new(Vec::new()));
    let fail_next = Arc::new(AtomicBool::new(false));
    let (stop_tx, stop) = oneshot::channel();
    let failed_opens = Arc::new(AtomicUsize::new(0));
    let server = tonic::transport::Server::builder()
        .add_service(RaftServiceServer::new(Peer {
            inner: RaftServiceImpl::new(follower.clone()),
            raft: follower.clone(),
            close: close.clone(),
            fail_open: fail_next.clone(),
            failed_opens: failed_opens.clone(),
        }))
        .serve_with_shutdown(address, async {
            let _ = stop.await;
        });
    let server = TypeConfig::spawn(server);
    TypeConfig::timeout(TIMEOUT, async {
        loop {
            if tonic::transport::Endpoint::from_shared(format!("http://{address}"))
                .unwrap()
                .connect()
                .await
                .is_ok()
            {
                break;
            }
            TypeConfig::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let leader = Raft::new(
        1,
        config,
        network,
        LogStore::default(),
        Arc::new(StateMachineStore::default()),
    )
    .await
    .unwrap();
    leader.initialize(btreemap! {1 => pb::Node { node_id: 1, rpc_addr: String::new() }}).await.unwrap();
    leader
        .wait(Some(TIMEOUT))
        .metrics(
            |m| {
                m.current_leader == Some(1)
                    && m.cluster_committed.as_ref().is_some_and(|id| id.leader_id == m.current_term)
            },
            "initial leader log committed before changing membership",
        )
        .await
        .unwrap();
    let add = leader.add_learner(
        2,
        pb::Node {
            node_id: 2,
            rpc_addr: address.to_string(),
        },
        true,
    );
    TypeConfig::timeout(TIMEOUT, add).await.unwrap().unwrap();
    leader.change_membership(btreeset! {1, 2}, false).await.unwrap();

    tracing::info!("--- stream 64 writes of 4 KiB, then close the idle response half");
    for i in 0..64 {
        leader
            .client_write(pb::SetRequest {
                key: format!("key-{i}"),
                value: "x".repeat(4096),
            })
            .await
            .unwrap();
    }
    let first = streams.lock().unwrap().iter().find(|s| s.state.load(Ordering::SeqCst) == 2).unwrap().clone();
    let before = usize::from(first.state.load(Ordering::SeqCst) != 0);
    fail_next.store(fail_open, Ordering::SeqCst);
    {
        let mut close = close.lock().unwrap();
        let index = close.iter().position(|tx| !tx.is_canceled()).unwrap();
        close.remove(index).send(()).unwrap();
    }
    let released = TypeConfig::timeout(TIMEOUT, async {
        loop {
            if first.state.load(Ordering::SeqCst) == 0 {
                break;
            }
            TypeConfig::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let after = usize::from(first.state.load(Ordering::SeqCst) != 0);
    let write = TypeConfig::timeout(
        TIMEOUT,
        leader.client_write(pb::SetRequest {
            key: "after-close".into(),
            value: "y".repeat(4096),
        }),
    )
    .await;
    let failed_released = TypeConfig::timeout(TIMEOUT, async {
        loop {
            let done = {
                let streams = streams.lock().unwrap();
                let failed: Vec<_> = streams.iter().filter(|s| s.open_failed.load(Ordering::SeqCst)).collect();
                (!fail_open || !failed.is_empty()) && failed.iter().all(|s| s.state.load(Ordering::SeqCst) == 0)
            };
            if done {
                break;
            }
            TypeConfig::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    eprintln!(
        "fail_open={fail_open}: ended request stream retained before={before}, after={after}, failed_opens={}",
        failed_opens.load(Ordering::SeqCst)
    );
    leader.shutdown().await.unwrap();
    follower.shutdown().await.unwrap();
    let _ = stop_tx.send(());
    server.await.unwrap().unwrap();
    assert_eq!(
        failed_opens.load(Ordering::SeqCst),
        usize::from(fail_open),
        "the requested open failure ran"
    );
    assert!(
        failed_released.is_ok(),
        "failed tonic open retains its queued request stream"
    );
    assert_eq!(before, 1, "the fault must end an active pipeline");
    assert!(released.is_ok(), "ended tonic session retains its request stream");
    assert!(
        matches!(write, Ok(Ok(_))),
        "client write stalls after the tonic session ends: {write:?}"
    );
}

#[test]
fn ended_tonic_sessions_release_requests() {
    TypeConfig::run(async {
        check_session(false).await;
        check_session(true).await;
    });
}
