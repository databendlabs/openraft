//! Send the empty AppendEntries requests of one node's groups to each target node in batches.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::channel::oneshot;
use openraft::error::Unreachable;
use openraft::type_config::TypeConfigExt;

use crate::GroupId;
use crate::NodeId;
use crate::TypeConfig;
use crate::router::Router;
use crate::router::RouterError;
use crate::typ::AppendEntriesRequest;
use crate::typ::AppendEntriesResponse;
use crate::typ::RaftError;

/// How long a batch waits for more requests after its first one.
const MAX_DELAY: Duration = Duration::from_millis(1);

/// The most requests one batch carries.
const MAX_REQUESTS: usize = 1024;

/// What a caller of [`AppendBatcher::append()`] gets back.
type AppendResponse = Result<AppendEntriesResponse, Unreachable<TypeConfig>>;

/// A request waiting for its batch, with the channel its caller waits on.
struct Pending {
    target: NodeId,
    group_id: GroupId,
    rpc: AppendEntriesRequest,
    response_tx: oneshot::Sender<AppendResponse>,
}

/// How many batches an [`AppendBatcher`] has sent, and how many requests they carried.
#[derive(Debug, Default)]
pub struct BatchStats {
    pub batches: AtomicU64,
    pub requests: AtomicU64,
}

/// Sends the empty AppendEntries requests of one node's groups, mostly heartbeats, to each target
/// node in batches.
///
/// A task collects the requests that arrive within [`MAX_DELAY`] of the first one. It sends the
/// requests for each target node as one `/raft/append-batch` message, whose reply holds one
/// result per request, in request order. Each caller gets its own result through a oneshot
/// channel.
#[derive(Clone)]
pub struct AppendBatcher {
    tx: mpsc::Sender<Pending>,
    stats: Arc<BatchStats>,
}

impl AppendBatcher {
    /// Start the task that sends the batches through `router`.
    pub fn spawn(router: Router) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        let stats = Arc::new(BatchStats::default());
        TypeConfig::spawn(collect_batches(router, rx, stats.clone()));
        Self { tx, stats }
    }

    /// Send `rpc` of `group_id` to `target` in the next batch, and return its response.
    pub async fn append(&self, target: NodeId, group_id: GroupId, rpc: AppendEntriesRequest) -> AppendResponse {
        let (response_tx, response_rx) = oneshot::channel();
        let pending = Pending {
            target,
            group_id,
            rpc,
            response_tx,
        };

        self.tx.clone().send(pending).await.map_err(|e| unreachable_error(e.to_string()))?;
        response_rx.await.map_err(|e| unreachable_error(e.to_string()))?
    }

    pub fn stats(&self) -> &BatchStats {
        &self.stats
    }
}

fn unreachable_error(message: String) -> Unreachable<TypeConfig> {
    Unreachable::new(&RouterError(message))
}

/// Collect the requests into batches, one per target node, until every [`AppendBatcher`] is
/// dropped.
async fn collect_batches(router: Router, mut rx: mpsc::Receiver<Pending>, stats: Arc<BatchStats>) {
    while let Some(first) = rx.next().await {
        let flush_at = TypeConfig::now() + MAX_DELAY;
        let mut requests = vec![first];

        while requests.len() < MAX_REQUESTS {
            match TypeConfig::timeout_at(flush_at, rx.next()).await {
                Ok(Some(pending)) => requests.push(pending),
                // The delay is over, or every sender is gone.
                Err(_) | Ok(None) => break,
            }
        }

        let mut batches = BTreeMap::<NodeId, Vec<Pending>>::new();
        for pending in requests {
            batches.entry(pending.target).or_default().push(pending);
        }

        // Each batch is sent by its own task, so that a slow node holds up neither the batches to
        // other nodes nor the collection of the next batch.
        for (target, batch) in batches {
            stats.batches.fetch_add(1, Ordering::Relaxed);
            stats.requests.fetch_add(batch.len() as u64, Ordering::Relaxed);
            TypeConfig::spawn(send_batch(router.clone(), target, batch));
        }
    }
}

/// Send `batch` to `target`, and pass each result to the caller of its request.
async fn send_batch(router: Router, target: NodeId, batch: Vec<Pending>) {
    let (requests, response_txs): (Vec<_>, Vec<_>) =
        batch.into_iter().map(|p| ((p.group_id, p.rpc), p.response_tx)).unzip();

    let sent: Result<Vec<Result<AppendEntriesResponse, RaftError>>, _> =
        router.send_to_node(target, "/raft/append-batch", requests).await;

    let results: Vec<AppendResponse> = match sent {
        Ok(results) if results.len() == response_txs.len() => {
            results.into_iter().map(|res| res.map_err(|e| unreachable_error(e.to_string()))).collect()
        }
        // Without one result per request, no result can be matched to its request.
        Ok(results) => {
            let message = format!(
                "node {target} sent {} results for {} requests",
                results.len(),
                response_txs.len()
            );
            response_txs.iter().map(|_| Err(unreachable_error(message.clone()))).collect()
        }
        Err(e) => response_txs.iter().map(|_| Err(unreachable_error(e.to_string()))).collect(),
    };

    for (response_tx, result) in response_txs.into_iter().zip(results) {
        // A caller that gave up has dropped its receiver and needs no result.
        let _ = response_tx.send(result);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    use futures::StreamExt;
    use futures::channel::mpsc;
    use futures::future::join_all;
    use openraft::type_config::TypeConfigExt;

    use super::AppendBatcher;
    use crate::GroupId;
    use crate::NodeId;
    use crate::TypeConfig;
    use crate::decode;
    use crate::encode;
    use crate::router::Router;
    use crate::typ::AppendEntriesRequest;
    use crate::typ::AppendEntriesResponse;
    use crate::typ::RaftError;
    use crate::typ::Vote;

    /// The group ids of each batch a fake node received.
    type Received = Arc<Mutex<Vec<Vec<GroupId>>>>;

    /// Register on `router` a fake node `node_id`, which answers each request of a batch with
    /// `answer(group_id)`.
    fn fake_node(router: &Router, node_id: NodeId, answer: fn(&str) -> AppendEntriesResponse) -> Received {
        let (tx, mut rx) = mpsc::channel(16);
        router.register_node(node_id, tx);

        let received = Received::default();
        let recorded = received.clone();
        TypeConfig::spawn(async move {
            while let Some(msg) = rx.next().await {
                let requests: Vec<(GroupId, AppendEntriesRequest)> = decode(&msg.payload);
                let groups: Vec<GroupId> = requests.into_iter().map(|(group_id, _)| group_id).collect();
                let results: Vec<Result<_, RaftError>> = groups.iter().map(|g| Ok(answer(g))).collect();

                recorded.lock().unwrap().push(groups);
                let _ = msg.response_tx.send(encode(Ok::<_, RaftError>(results)));
            }
        });
        received
    }

    fn heartbeat() -> AppendEntriesRequest {
        AppendEntriesRequest {
            vote: Vote::new_committed(1, 1),
            prev_log_id: None,
            entries: vec![],
            leader_commit: None,
        }
    }

    #[test]
    fn test_requests_to_one_node_share_a_batch() {
        TypeConfig::run(async {
            let router = Router::new();
            let received = fake_node(&router, 2, |group_id| match group_id {
                "orders" => AppendEntriesResponse::Conflict,
                _ => AppendEntriesResponse::Success,
            });
            let batcher = AppendBatcher::spawn(router);

            let groups = ["users", "orders", "products"].map(String::from);
            let calls = groups.clone().map(|group_id| batcher.append(2, group_id, heartbeat()));
            let results: Vec<_> = join_all(calls).await.into_iter().map(Result::unwrap).collect();

            let expected = [
                AppendEntriesResponse::Success,
                AppendEntriesResponse::Conflict,
                AppendEntriesResponse::Success,
            ];
            assert_eq!(expected.to_vec(), results);
            assert_eq!(vec![groups.to_vec()], *received.lock().unwrap());
        });
    }

    #[test]
    fn test_each_target_node_gets_its_own_batch() {
        TypeConfig::run(async {
            let router = Router::new();
            let received_by_2 = fake_node(&router, 2, |_| AppendEntriesResponse::Success);
            let received_by_3 = fake_node(&router, 3, |_| AppendEntriesResponse::Success);
            let batcher = AppendBatcher::spawn(router);

            let to_2 = batcher.append(2, "users".to_string(), heartbeat());
            let to_3 = batcher.append(3, "orders".to_string(), heartbeat());
            let (from_2, from_3) = futures::join!(to_2, to_3);

            assert_eq!(AppendEntriesResponse::Success, from_2.unwrap());
            assert_eq!(AppendEntriesResponse::Success, from_3.unwrap());
            assert_eq!(vec![vec!["users".to_string()]], *received_by_2.lock().unwrap());
            assert_eq!(vec![vec!["orders".to_string()]], *received_by_3.lock().unwrap());
            assert_eq!(2, batcher.stats().batches.load(Ordering::Relaxed));
        });
    }

    #[test]
    fn test_a_failed_batch_fails_every_request() {
        TypeConfig::run(async {
            // No node 2 is registered on the router, so the batch to it fails.
            let batcher = AppendBatcher::spawn(Router::new());

            let calls = ["users", "orders"].map(|group_id| batcher.append(2, group_id.to_string(), heartbeat()));
            let results = join_all(calls).await;

            assert!(results.iter().all(|res| res.is_err()), "got: {results:?}");
        });
    }
}
