use std::future::Future;
use std::num::NonZeroUsize;

use futures_util::Stream;
use futures_util::StreamExt;
use openraft::RaftTypeConfig;
use openraft::error::RPCError;
use openraft::openraft_macros::since;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;

use crate::BatchItem;
use crate::BatchReply;

/// Run every item of a received batch against its Raft group, and yield each reply as soon as its
/// group answers.
///
/// `append` passes one request to `Raft::append_entries()` of the named group. For a group that
/// this node does not have, it returns an error such as `Unreachable`, which makes the sender back
/// off. At most `concurrency` items run at a time, and a group that blocks delays only its own
/// reply.
#[since(version = "0.10.1")]
pub fn dispatch_batch<C, G, F, Fut>(
    items: Vec<BatchItem<C, G>>,
    concurrency: NonZeroUsize,
    mut append: F,
) -> impl Stream<Item = BatchReply<C, G>>
where
    C: RaftTypeConfig,
    G: Clone,
    F: FnMut(G, AppendEntriesRequest<C>) -> Fut,
    Fut: Future<Output = Result<AppendEntriesResponse<C>, RPCError<C>>>,
{
    futures_util::stream::iter(items)
        .map(move |BatchItem { id, group_id, rpc }| {
            let response = append(group_id.clone(), rpc);
            async move {
                BatchReply {
                    id,
                    group_id,
                    result: response.await,
                }
            }
        })
        .buffer_unordered(concurrency.get())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use futures_util::StreamExt;
    use futures_util::lock::Mutex;
    use openraft::error::RPCError;
    use openraft::error::Unreachable;
    use openraft::raft::AppendEntriesResponse;
    use openraft::type_config::TypeConfigExt;

    use crate::BatchItem;
    use crate::dispatch_batch;
    use crate::testing::TestConfig;
    use crate::testing::heartbeat;

    type C = TestConfig;

    #[test]
    fn test_replies_come_as_each_group_answers() {
        C::run(async {
            let gate = Arc::new(Mutex::new(()));
            let held = gate.lock().await;

            let items = (1..=3).map(|group_id| BatchItem {
                id: group_id * 10,
                group_id,
                rpc: heartbeat(group_id),
            });
            let replies = dispatch_batch(items.collect(), NonZeroUsize::new(3).unwrap(), |group_id, _rpc| {
                let gate = gate.clone();
                async move {
                    match group_id {
                        1 => {
                            let _open = gate.lock().await;
                            Ok(AppendEntriesResponse::Success)
                        }
                        2 => Ok(AppendEntriesResponse::Success),
                        _ => Err(RPCError::Unreachable(Unreachable::from_string("unknown group"))),
                    }
                }
            });
            let mut replies = Box::pin(replies);

            tracing::info!("--- group 1 blocks; the replies of groups 2 and 3 come first");
            {
                let first = replies.next().await.unwrap();
                let second = replies.next().await.unwrap();
                let ids = BTreeSet::from([first.id, second.id]);
                assert_eq!(BTreeSet::from([20, 30]), ids);

                for reply in [first, second] {
                    match reply.group_id {
                        2 => assert_eq!(Ok(AppendEntriesResponse::Success), reply.result),
                        _ => assert!(matches!(reply.result, Err(RPCError::Unreachable(_)))),
                    }
                }
            }

            tracing::info!("--- unblock group 1; its reply ends the stream");
            {
                drop(held);
                let third = replies.next().await.unwrap();
                assert_eq!((10, 1), (third.id, third.group_id));
                assert_eq!(Ok(AppendEntriesResponse::Success), third.result);
                assert!(replies.next().await.is_none());
            }
        });
    }
}
