use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::future::join_all;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::base::BoxStream;
use openraft::entry::RaftEntry;
use openraft::error::RPCError;
use openraft::error::ReplicationClosed;
use openraft::error::StreamingError;
use openraft::error::Unreachable;
use openraft::impls::Vote;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::AppendError;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::VoteOf;

use crate::BatchConfig;
use crate::BatchItem;
use crate::BatchReply;
use crate::BatchTransport;
use crate::BatchingRouter;
use crate::GroupRouter;
use crate::testing::TestConfig;
use crate::testing::appended;
use crate::testing::heartbeat;

type C = TestConfig;

type Answer = Result<Vec<BatchReply<C, u64>>, RPCError<C>>;

/// How a test transport answers a batch.
type AnswerFn = Arc<dyn Fn(&[BatchItem<C, u64>]) -> Answer + Send + Sync>;

/// The target node and the group ids of every batch a test transport sent.
type SentBatches = Arc<Mutex<Vec<(u64, Vec<u64>)>>>;

const TTL: Duration = Duration::from_secs(1);

/// The router behind the batching one: it records each request and fails it as `inner_error()`.
#[derive(Clone, Default)]
struct Inner {
    received: Arc<Mutex<Vec<String>>>,
}

impl Inner {
    fn reject(&self, request: String) {
        self.received.lock().unwrap().push(request);
    }
}

fn inner_error() -> RPCError<C> {
    RPCError::Unreachable(Unreachable::from_string("inner"))
}

impl GroupRouter<C, u64> for Inner {
    type SnapshotData = std::io::Cursor<Vec<u8>>;

    async fn append_entries(
        &self,
        target: u64,
        group_id: u64,
        rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        self.reject(format!(
            "append to {target}/{group_id} with {} entries",
            rpc.entries.len()
        ));
        Err(inner_error())
    }

    async fn vote(
        &self,
        target: u64,
        group_id: u64,
        _rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        self.reject(format!("vote to {target}/{group_id}"));
        Err(inner_error())
    }

    async fn pre_vote(
        &self,
        target: u64,
        group_id: u64,
        _rpc: VoteRequest<C>,
        _option: RPCOption,
    ) -> Result<VoteResponse<C>, RPCError<C>> {
        self.reject(format!("pre_vote to {target}/{group_id}"));
        Err(inner_error())
    }

    async fn full_snapshot(
        &self,
        target: u64,
        group_id: u64,
        _vote: VoteOf<C>,
        _snapshot: SnapshotOf<C, Self::SnapshotData>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        self.reject(format!("snapshot to {target}/{group_id}"));
        Err(StreamingError::Unreachable(Unreachable::from_string("inner")))
    }
}

/// A transport that records the groups of every batch it sends, and answers it with `answer`.
///
/// While a test holds `gate`, the transport waits before it answers.
#[derive(Clone)]
struct Transport {
    sent: SentBatches,
    answer: AnswerFn,
    gate: Arc<futures_util::lock::Mutex<()>>,
}

impl Transport {
    fn new(answer: impl Fn(&[BatchItem<C, u64>]) -> Answer + Send + Sync + 'static) -> Self {
        Self {
            sent: Default::default(),
            answer: Arc::new(answer),
            gate: Default::default(),
        }
    }

    /// The transport that answers every item with success.
    fn succeeding() -> Self {
        Self::new(|items| Ok(items.iter().map(success).collect()))
    }

    fn sent(&self) -> Vec<(u64, Vec<u64>)> {
        self.sent.lock().unwrap().clone()
    }
}

impl BatchTransport<C, u64> for Transport {
    async fn send_batch(
        &self,
        target: u64,
        items: Vec<BatchItem<C, u64>>,
        _option: RPCOption,
    ) -> Result<BoxStream<'static, BatchReply<C, u64>>, RPCError<C>> {
        let groups = items.iter().map(|item| item.group_id).collect();
        self.sent.lock().unwrap().push((target, groups));
        let _open = self.gate.lock().await;
        let replies = (self.answer)(&items)?;
        Ok(Box::pin(futures_util::stream::iter(replies)))
    }
}

fn success(item: &BatchItem<C, u64>) -> BatchReply<C, u64> {
    BatchReply {
        id: item.id,
        group_id: item.group_id,
        result: Ok(AppendEntriesResponse::Success),
    }
}

/// A batching router over `transport` with `max_items` and a long `max_delay`, so that the
/// concurrent calls of one test share a batch.
fn router(transport: &Transport, max_items: usize) -> BatchingRouter<C, u64, Inner, Transport> {
    let config = BatchConfig {
        max_items: NonZeroUsize::new(max_items).unwrap(),
        max_delay: Duration::from_millis(50),
        ..Default::default()
    };
    BatchingRouter::new(Inner::default(), transport.clone(), config)
}

/// Send a heartbeat of each of `groups` to `target` at once, and return their results in order.
async fn heartbeats(
    router: &BatchingRouter<C, u64, Inner, Transport>,
    target: u64,
    groups: impl IntoIterator<Item = u64>,
) -> Vec<Result<AppendResult<C>, RPCError<C>>> {
    let calls = groups
        .into_iter()
        .map(|group_id| router.append_entries(target, group_id, heartbeat(group_id), RPCOption::new(TTL)));
    join_all(calls).await
}

#[test]
fn test_empty_appends_to_one_peer_share_a_batch() {
    C::run(async {
        let transport = Transport::succeeding();
        let router = router(&transport, 16);

        tracing::info!("--- three groups send a heartbeat to node 1 within the batching delay");
        let results = heartbeats(&router, 1, 1..=3).await;

        assert_eq!(vec![Ok(appended(1)), Ok(appended(2)), Ok(appended(3))], results);
        assert_eq!(vec![(1, vec![1, 2, 3])], transport.sent());
        assert!(router.inner().received.lock().unwrap().is_empty());
    });
}

#[test]
fn test_replies_are_matched_by_id_in_any_order() {
    C::run(async {
        let higher = Vote::new_committed(2, 1);
        let transport = Transport::new(move |items| {
            let reversed = items.iter().rev();
            let replies = reversed.map(|item| match item.group_id {
                2 => BatchReply {
                    result: Ok(AppendEntriesResponse::HigherVote(higher)),
                    ..success(item)
                },
                _ => success(item),
            });
            Ok(replies.collect())
        });
        let router = router(&transport, 16);

        tracing::info!("--- the transport answers in reverse order, with a higher vote for group 2");
        let results = heartbeats(&router, 1, 1..=3).await;

        let higher_vote = Err(AppendError::HigherVote(higher));
        assert_eq!(vec![Ok(appended(1)), Ok(higher_vote), Ok(appended(3))], results);
    });
}

#[test]
fn test_a_missing_reply_fails_only_its_item() {
    C::run(async {
        let transport = Transport::new(|items| {
            let mut replies: Vec<_> = items.iter().filter(|item| item.group_id != 2).map(success).collect();
            tracing::info!("--- add a reply with an unknown id, and repeat the reply of group 1");
            replies.push(BatchReply {
                id: u64::MAX,
                ..success(&items[0])
            });
            replies.push(success(&items[0]));
            Ok(replies)
        });
        let router = router(&transport, 16);

        tracing::info!("--- the transport answers groups 1 and 3, but not group 2");
        let results = heartbeats(&router, 1, 1..=3).await;

        assert_eq!(Ok(appended(1)), results[0]);
        assert!(matches!(results[1], Err(RPCError::Network(_))), "got: {:?}", results[1]);
        assert_eq!(Ok(appended(3)), results[2]);
    });
}

#[test]
fn test_a_transport_error_fails_every_item() {
    C::run(async {
        let down = RPCError::Unreachable(Unreachable::from_string("node 1 is down"));
        let transport = Transport::new({
            let down = down.clone();
            move |_items| Err(down.clone())
        });
        let router = router(&transport, 16);

        tracing::info!("--- the transport fails the batch of three groups");
        let results = heartbeats(&router, 1, 1..=3).await;

        assert_eq!(vec![Err(down.clone()), Err(down.clone()), Err(down)], results);
    });
}

#[test]
fn test_batches_are_split_by_size_and_by_peer() {
    C::run(async {
        let transport = Transport::succeeding();
        let router = router(&transport, 2);

        tracing::info!("--- three groups to node 1 fill two batches of at most two items");
        {
            let results = heartbeats(&router, 1, 1..=3).await;
            assert_eq!(vec![Ok(appended(1)), Ok(appended(2)), Ok(appended(3))], results);
            assert_eq!(vec![(1, vec![1, 2]), (1, vec![3])], transport.sent());
        }

        tracing::info!("--- node 2 gets a batch of its own");
        {
            let results = heartbeats(&router, 2, [4]).await;
            assert_eq!(vec![Ok(appended(4))], results);
            assert_eq!(Some((2, vec![4])), transport.sent().last().cloned());
        }
    });
}

#[test]
fn test_requests_that_are_not_empty_appends_bypass_batches() {
    C::run(async {
        let transport = Transport::succeeding();
        let router = router(&transport, 16);

        tracing::info!("--- an append with entries, a vote and a pre-vote go to the inner router");
        {
            let mut rpc = heartbeat(1);
            rpc.entries.push(<C as RaftTypeConfig>::Entry::new_blank(log_id::<C>(1, 0, 2)));
            let appended = router.append_entries(1, 1, rpc, RPCOption::new(TTL)).await;
            assert_eq!(Err(inner_error()), appended);

            let request = VoteRequest::new(Vote::new(2, 0), None);
            let voted = router.vote(1, 2, request.clone(), RPCOption::new(TTL)).await;
            assert!(voted.is_err());
            let pre_voted = router.pre_vote(1, 3, request, RPCOption::new(TTL)).await;
            assert!(pre_voted.is_err());
        }

        let received = router.inner().received.lock().unwrap().clone();
        let expected = ["append to 1/1 with 1 entries", "vote to 1/2", "pre_vote to 1/3"];
        assert_eq!(expected.map(String::from).to_vec(), received);
        assert!(transport.sent().is_empty());
    });
}

#[test]
fn test_a_cancelled_call_leaves_its_batch_intact() {
    C::run(async {
        let transport = Transport::succeeding();
        let router = router(&transport, 16);

        tracing::info!("--- hold the transport, then send heartbeats of groups 1 and 2 in one batch");
        let held = transport.gate.lock().await;
        let calls = [1, 2].map(|group_id| {
            let router = router.clone();
            C::spawn(async move { router.append_entries(1, group_id, heartbeat(group_id), RPCOption::new(TTL)).await })
        });
        let [call_1, call_2] = calls;
        C::timeout(TTL, async {
            while transport.sent().is_empty() {
                C::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(vec![(1, vec![1, 2])], transport.sent());

        tracing::info!("--- cancel the call of group 1 while its batch waits for replies");
        {
            call_1.abort();
            drop(held);

            let result = C::timeout(TTL, call_2).await.unwrap().unwrap();
            assert_eq!(Ok(appended(2)), result);
        }
    });
}
