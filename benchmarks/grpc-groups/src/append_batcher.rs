use std::collections::BTreeMap;
use std::time::Duration;

use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use raft_kv_memstore_grpc::protobuf as raftpb;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tonic::Code;
use tonic::Status;

use crate::C;
use crate::protobuf as pb;
use crate::protobuf::group_append_result::Result as BatchResult;
use crate::router::Router;
use crate::router::rpc_error;

/// How long a batch waits for more requests after its first one.
const MAX_DELAY: Duration = Duration::from_millis(1);

/// The most requests that one batch carries.
const MAX_REQUESTS: usize = 1024;

/// The reply to one request of a batch, or why it got none.
type Reply = Result<raftpb::AppendEntriesResponse, RPCError<C>>;

/// A request that waits for its batch, with the channel on which its caller waits for the reply.
struct Pending {
    target: u64,
    request: pb::GroupAppendRequest,
    reply_tx: oneshot::Sender<Reply>,
}

/// Sends the AppendEntries requests without entries of this node's groups, mostly heartbeats, to
/// each target node in batches.
///
/// A task collects the requests that arrive within [`MAX_DELAY`] of the first one, up to
/// [`MAX_REQUESTS`]. It sends those for each target node in one `AppendBatch` call, whose reply
/// holds one result per request, in request order, and passes each caller its own result.
#[derive(Clone)]
pub(crate) struct AppendBatcher {
    tx: mpsc::Sender<Pending>,
}

impl AppendBatcher {
    /// Start the task that sends the batches through `router`, which must not batch itself.
    pub(crate) fn spawn(router: Router) -> Self {
        let (tx, rx) = mpsc::channel(MAX_REQUESTS);
        tokio::spawn(collect_batches(router, rx));
        Self { tx }
    }

    /// Send `request` to `target` in the next batch, and return its reply.
    pub(crate) async fn append(&self, target: u64, request: pb::GroupAppendRequest) -> Reply {
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = Pending {
            target,
            request,
            reply_tx,
        };

        self.tx.send(pending).await.map_err(|_| no_reply("the batcher has stopped"))?;
        reply_rx.await.map_err(|_| no_reply("its batch ended without a result for it"))?
    }
}

/// Collect the requests into batches, one per target node, until every [`AppendBatcher`] is
/// dropped.
async fn collect_batches(router: Router, mut rx: mpsc::Receiver<Pending>) {
    while let Some(first) = rx.recv().await {
        let flush_at = Instant::now() + MAX_DELAY;
        let mut pendings = vec![first];

        while pendings.len() < MAX_REQUESTS {
            match tokio::time::timeout_at(flush_at, rx.recv()).await {
                Ok(Some(pending)) => pendings.push(pending),
                // The delay is over, or every sender is gone.
                Err(_) | Ok(None) => break,
            }
        }

        let mut batches = BTreeMap::<u64, Vec<Pending>>::new();
        for pending in pendings {
            batches.entry(pending.target).or_default().push(pending);
        }

        // Each batch is sent by its own task, so that a slow node holds up neither the batches to
        // the other nodes nor the collection of the next batch.
        for (target, batch) in batches {
            tokio::spawn(send_batch(router.clone(), target, batch));
        }
    }
}

/// Send `batch` to `target` in one call, and pass each caller the result of its request.
async fn send_batch(router: Router, target: u64, batch: Vec<Pending>) {
    let (requests, reply_txs): (Vec<_>, Vec<_>) = batch.into_iter().map(|p| (p.request, p.reply_tx)).unzip();

    let sent = router.append_batch(target, requests).await;

    let count = reply_txs.len();
    for (reply_tx, reply) in reply_txs.into_iter().zip(replies(sent, count)) {
        // A caller that gave up, at its timeout, has dropped its receiver and needs no reply.
        reply_tx.send(reply).ok();
    }
}

/// Split what a batch of `count` requests got into the reply to each request.
fn replies(sent: Result<Vec<pb::GroupAppendResult>, RPCError<C>>, count: usize) -> Vec<Reply> {
    let results = match sent {
        Ok(results) if results.len() == count => results,
        // Without one result per request, no result can be matched to its request.
        Ok(results) => {
            let error = no_reply(&format!("{count} requests of a batch got {} results", results.len()));
            return vec![Err(error); count];
        }
        Err(error) => return vec![Err(error); count],
    };

    let reply = |result: pb::GroupAppendResult| match result.result {
        Some(BatchResult::Response(response)) => Ok(response),
        // The node failed this request alone, as it would have failed a call of its own.
        Some(BatchResult::Error(error)) => Err(rpc_error(Status::new(Code::from(error.code), error.message))),
        None => Err(no_reply("its result in the batch is empty")),
    };
    results.into_iter().map(reply).collect()
}

/// The error of a request that got no reply, for `reason`.
fn no_reply(reason: &str) -> RPCError<C> {
    RPCError::Network(NetworkError::from_string(format!("no reply: {reason}")))
}

#[cfg(test)]
mod tests {
    use openraft::errors::RPCError;
    use openraft::errors::Unreachable;
    use raft_kv_memstore_grpc::protobuf as raftpb;
    use tonic::Code;

    use super::replies;
    use crate::protobuf as pb;
    use crate::protobuf::group_append_result::Result as BatchResult;

    fn response(last_log_index: u64) -> pb::GroupAppendResult {
        let response = raftpb::AppendEntriesResponse {
            last_log_id: Some(raftpb::LogId {
                term: 1,
                index: last_log_index,
            }),
            ..Default::default()
        };
        pb::GroupAppendResult {
            result: Some(BatchResult::Response(response)),
        }
    }

    fn error(code: Code) -> pb::GroupAppendResult {
        let error = pb::GroupAppendError {
            code: code as i32,
            message: format!("{code:?}"),
        };
        pb::GroupAppendResult {
            result: Some(BatchResult::Error(error)),
        }
    }

    #[test]
    fn test_each_request_gets_its_own_result() {
        let results = vec![
            response(3),
            error(Code::NotFound),
            error(Code::Unavailable),
            response(5),
        ];

        let replies = replies(Ok(results), 4);

        let index = |reply: &super::Reply| reply.as_ref().unwrap().last_log_id.as_ref().unwrap().index;
        assert_eq!(3, index(&replies[0]));
        assert!(matches!(replies[1], Err(RPCError::Network(_))), "got: {:?}", replies[1]);
        assert!(
            matches!(replies[2], Err(RPCError::Unreachable(_))),
            "got: {:?}",
            replies[2]
        );
        assert_eq!(5, index(&replies[3]));
    }

    #[test]
    fn test_a_batch_without_one_result_per_request_fails_each_request() {
        let replies = replies(Ok(vec![response(3)]), 2);

        assert_eq!(2, replies.len());
        assert!(
            replies.iter().all(|reply| matches!(reply, Err(RPCError::Network(_)))),
            "got: {replies:?}"
        );
    }

    #[test]
    fn test_a_failed_call_fails_each_request() {
        let unreachable = RPCError::Unreachable(Unreachable::from_string("node 2 is down"));

        let replies = replies(Err(unreachable.clone()), 3);

        assert_eq!(vec![Err(unreachable); 3], replies);
    }
}
