//! Batching receiver for RaftMsg that merges consecutive ClientWrite messages.
//!
//! This module provides [`BatchRaftMsgReceiver`], a wrapper around an mpsc receiver
//! that automatically batches consecutive `RaftMsg::ClientWrite` messages with the
//! same `expected_leader` into a single message, improving throughput by reducing
//! per-message overhead.

use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::TryRecvError;
use crate::base::RaftBatch;
use crate::core::raft_msg::RaftMsg;
use crate::error::Fatal;
use crate::type_config::alias::MpscReceiverOf;

/// A receiver wrapper that batches consecutive `RaftMsg::ClientWrite` messages.
///
/// When receiving messages, this receiver merges consecutive `ClientWrite` messages
/// that have the same `expected_leader` value into a single message. This batching
/// reduces per-message processing overhead and improves write throughput.
///
/// # Batching Rules
///
/// - Only `RaftMsg::ClientWrite` messages are batched
/// - Messages are only merged if they have the same `expected_leader` value
/// - Non-`ClientWrite` messages stop the batching and are buffered for the next recv
/// - Maximum batch size is 4096 messages
///
/// # Usage Pattern
///
/// ```ignore
/// // Wait for at least one message
/// receiver.ensure_buffered().await?;
///
/// // Process all available messages (with batching)
/// while let Some(msg) = receiver.try_recv()? {
///     handle_msg(msg);
/// }
/// ```
pub(crate) struct BatchRaftMsgReceiver<C>
where C: RaftTypeConfig
{
    /// A message that was received but not yet returned to the caller.
    ///
    /// This is used when:
    /// - A non-mergeable message is encountered during batching
    /// - A `ClientWrite` with different `expected_leader` is encountered
    buffered: Option<RaftMsg<C>>,

    /// The underlying mpsc receiver.
    inner: MpscReceiverOf<C, RaftMsg<C>>,
}

impl<C> BatchRaftMsgReceiver<C>
where C: RaftTypeConfig
{
    /// Creates a new batching receiver wrapping the given mpsc receiver.
    pub(crate) fn new(receiver: MpscReceiverOf<C, RaftMsg<C>>) -> Self {
        Self {
            buffered: None,
            inner: receiver,
        }
    }

    /// Waits until at least one message is available in the buffer.
    ///
    /// If the buffer is empty, this method blocks until a message arrives from the
    /// underlying receiver. If the buffer already contains a message, returns immediately.
    ///
    /// Returns `Err(Fatal::Stopped)` if the sender is dropped.
    pub(crate) async fn ensure_buffered(&mut self) -> Result<(), Fatal<C>> {
        if self.buffered.is_some() {
            return Ok(());
        }

        let msg = self.inner_recv().await?;
        self.buffered = Some(msg);

        Ok(())
    }

    /// Attempts to receive a message, merging consecutive `ClientWrite` messages.
    ///
    /// Returns `Ok(Some(msg))` if a message is available, `Ok(None)` if the channel
    /// is empty, or `Err(Fatal::Stopped)` if the sender is dropped.
    ///
    /// If the first available message is a `ClientWrite`, this method attempts to
    /// merge additional `ClientWrite` messages with the same `expected_leader`.
    pub(crate) fn try_recv(&mut self) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
        let msg = self.buffered_try_recv()?;

        let Some(mut msg) = msg else {
            return Ok(None);
        };

        self.merge_client_writes(&mut msg)?;

        Ok(Some(msg))
    }

    /// Returns a buffered message if available, otherwise tries the inner receiver.
    fn buffered_try_recv(&mut self) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
        if let Some(msg) = self.buffered.take() {
            return Ok(Some(msg));
        }

        self.inner_try_recv()
    }

    /// Waits for a message from the inner receiver.
    async fn inner_recv(&mut self) -> Result<RaftMsg<C>, Fatal<C>> {
        let Some(msg) = self.inner.recv().await else {
            tracing::info!("all rx_api senders are dropped");
            return Err(Fatal::Stopped);
        };

        Ok(msg)
    }

    /// Non-blocking receive from the inner receiver.
    fn inner_try_recv(&mut self) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
        let res = self.inner.try_recv();

        match res {
            Ok(msg) => Ok(Some(msg)),
            Err(e) => match e {
                TryRecvError::Empty => {
                    tracing::debug!("all RaftMsg are processed, wait for more");
                    Ok(None)
                }
                TryRecvError::Disconnected => {
                    tracing::debug!("rx_api is disconnected, quit");
                    Err(Fatal::Stopped)
                }
            },
        }
    }

    /// Merges consecutive `ClientWrite` messages with the same `expected_leader`.
    ///
    /// Reads additional messages from the inner receiver and merges them into `msg`
    /// if they are `ClientWrite` with matching `expected_leader`. Stops merging when:
    /// - A non-`ClientWrite` message is encountered (buffered for next recv)
    /// - A `ClientWrite` with different `expected_leader` is found (buffered for next recv)
    /// - Maximum batch size (4096) is reached
    /// - No more messages are available
    fn merge_client_writes(&mut self, msg: &mut RaftMsg<C>) -> Result<(), Fatal<C>> {
        debug_assert!(self.buffered.is_none());

        let (batch_payloads, batch_responders, batch_leader) = match msg {
            RaftMsg::ClientWrite {
                payloads,
                responders,
                expected_leader,
            } => (payloads, responders, expected_leader),
            _ => return Ok(()),
        };

        // TODO: make it configurable
        let max_batch_size = 4096;

        for _i in 0..max_batch_size {
            let next = self.inner_try_recv()?;

            let Some(next) = next else {
                break;
            };

            let RaftMsg::ClientWrite {
                payloads,
                responders,
                expected_leader,
            } = next
            else {
                self.buffered = Some(next);
                break;
            };

            if &expected_leader == batch_leader {
                batch_payloads.extend(payloads);
                batch_responders.extend(responders);
            } else {
                self.buffered = Some(RaftMsg::ClientWrite {
                    payloads,
                    responders,
                    expected_leader,
                });
                break;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_runtime::MpscSender;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::entry::EntryPayload;
    use crate::type_config::TypeConfigExt;
    use crate::type_config::alias::CommittedLeaderIdOf;

    type C = UTConfig<()>;

    fn committed_leader_id(term: u64, node_id: u64) -> CommittedLeaderIdOf<C> {
        *log_id(term, node_id, 0).committed_leader_id()
    }

    fn client_write(data: u64, leader: Option<CommittedLeaderIdOf<C>>) -> RaftMsg<C> {
        RaftMsg::ClientWrite {
            payloads: C::Batch::from_item(EntryPayload::Normal(data)),
            responders: C::Batch::from_item(None),
            expected_leader: leader,
        }
    }

    fn extract_payload_data(payloads: &C::Batch<EntryPayload<C>>) -> Vec<u64> {
        payloads
            .iter()
            .map(|p| match p {
                EntryPayload::Normal(d) => *d,
                _ => panic!("expected Normal payload"),
            })
            .collect()
    }

    #[test]
    fn test_merge_consecutive_client_writes_with_same_leader() {
        C::run(async {
            let (tx, rx) = C::mpsc(100);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx);

            let leader = Some(committed_leader_id(1, 1));
            tx.send(client_write(1, leader)).await.unwrap();
            tx.send(client_write(2, leader)).await.unwrap();
            tx.send(client_write(3, leader)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();
            let msg = receiver.try_recv().unwrap().unwrap();

            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![1, 2, 3]);
            assert_eq!(responders.len(), payloads.len());
        });
    }

    #[test]
    fn test_no_merge_when_expected_leader_differs() {
        C::run(async {
            let (tx, rx) = C::mpsc(100);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx);

            let leader1 = Some(committed_leader_id(1, 1));
            let leader2 = Some(committed_leader_id(2, 1));
            tx.send(client_write(1, leader1)).await.unwrap();
            tx.send(client_write(2, leader2)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();

            // First message should not be merged with second
            let msg1 = receiver.try_recv().unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg1
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![1]);
            assert_eq!(responders.len(), payloads.len());

            // Second message should be buffered and returned separately
            let msg2 = receiver.try_recv().unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg2
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![2]);
            assert_eq!(responders.len(), payloads.len());
        });
    }

    #[test]
    fn test_non_client_write_stops_merging() {
        C::run(async {
            let (tx, rx) = C::mpsc(100);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx);

            let leader = Some(committed_leader_id(1, 1));
            tx.send(client_write(1, leader)).await.unwrap();
            tx.send(RaftMsg::WithRaftState { req: Box::new(|_| {}) }).await.unwrap();
            tx.send(client_write(2, leader)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();

            // First ClientWrite should not merge past the WithRaftState
            let msg1 = receiver.try_recv().unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg1
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![1]);
            assert_eq!(responders.len(), payloads.len());

            // WithRaftState should be returned next
            let msg2 = receiver.try_recv().unwrap().unwrap();
            assert!(matches!(msg2, RaftMsg::WithRaftState { .. }));

            // Last ClientWrite should be returned
            let msg3 = receiver.try_recv().unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg3
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![2]);
            assert_eq!(responders.len(), payloads.len());
        });
    }

    #[test]
    fn test_try_recv_returns_none_when_empty() {
        C::run(async {
            let (_tx, rx) = C::mpsc::<RaftMsg<C>>(100);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx);

            let result = receiver.try_recv().unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_ensure_buffered_waits_for_message() {
        C::run(async {
            let (tx, rx) = C::mpsc(100);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx);

            tx.send(client_write(42, None)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();

            // Message should be in buffer, try_recv should return it
            let msg = receiver.try_recv().unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![42]);
            assert_eq!(responders.len(), payloads.len());
        });
    }

    #[test]
    fn test_ensure_buffered_returns_immediately_if_already_buffered() {
        C::run(async {
            let (tx, rx) = C::mpsc(100);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx);

            let leader = Some(committed_leader_id(1, 1));
            tx.send(client_write(1, None)).await.unwrap();
            tx.send(client_write(2, leader)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();
            // try_recv will buffer the second message (different leader)
            let _msg1 = receiver.try_recv().unwrap().unwrap();

            // ensure_buffered should return immediately since msg2 is buffered
            receiver.ensure_buffered().await.unwrap();

            let msg2 = receiver.try_recv().unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg2
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![2]);
            assert_eq!(responders.len(), payloads.len());
        });
    }
}
