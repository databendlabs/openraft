//! Batching receiver for RaftMsg that merges consecutive ClientWrite messages.
//!
//! This module provides [`BatchRaftMsgReceiver`], a wrapper around an mpsc receiver
//! that automatically batches consecutive `RaftMsg::ClientWrite` messages with the
//! same `expected_leader` into a single message, improving throughput by reducing
//! per-message overhead.

use std::ops::Add;
use std::time::Duration;

use rt::AsyncRuntime;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::TryRecvError;
use crate::core::raft_msg::RaftMsg;
use crate::errors::Fatal;
use crate::type_config::TypeConfigExt;
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
pub(crate) struct BatchRaftMsgReceiver<C, SD = ()>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    /// A message that was received but not yet returned to the caller.
    ///
    /// This is used when:
    /// - A non-mergeable message is encountered during batching
    /// - A `ClientWrite` with different `expected_leader` is encountered
    buffered: Option<RaftMsg<C, SD>>,

    /// Maximum items allowed to be merged before return the batch.
    capacity: u64,

    /// Maximum time allowed to wait until max capacity target is reached.
    linger: Duration,

    /// The underlying mpsc receiver.
    inner: MpscReceiverOf<C, RaftMsg<C, SD>>,
}

impl<C, SD> BatchRaftMsgReceiver<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    /// Creates a new batching receiver given:
    /// - mpsc receiver channel
    /// - capacity
    /// - linger
    pub(crate) fn new(receiver: MpscReceiverOf<C, RaftMsg<C, SD>>, capacity: u64, linger: Duration) -> Self {
        Self {
            buffered: None,
            capacity,
            linger,
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
    pub(crate) async fn try_recv(&mut self) -> Result<Option<RaftMsg<C, SD>>, Fatal<C>> {
        let msg = self.buffered_try_recv()?;

        let Some(mut msg) = msg else {
            return Ok(None);
        };

        self.merge_client_writes(&mut msg).await?;

        Ok(Some(msg))
    }

    /// Returns a buffered message if available, otherwise tries the inner receiver.
    fn buffered_try_recv(&mut self) -> Result<Option<RaftMsg<C, SD>>, Fatal<C>> {
        if let Some(msg) = self.buffered.take() {
            return Ok(Some(msg));
        }

        self.inner_try_recv()
    }

    /// Waits for a message from the inner receiver.
    async fn inner_recv(&mut self) -> Result<RaftMsg<C, SD>, Fatal<C>> {
        let Some(msg) = self.inner.recv().await else {
            tracing::info!("all rx_api senders are dropped");
            return Err(Fatal::Stopped);
        };

        Ok(msg)
    }

    /// Receives the next value from this receiver, if available.
    ///
    /// Returns `None` if:
    /// - the deadline is reached before a value is received
    ///
    /// Returns `Err(Fatal::Stopped)` if:
    /// - the channel is disconnected
    async fn inner_recv_timeout_at(
        &mut self,
        deadline: <C::AsyncRuntime as AsyncRuntime>::Instant,
    ) -> Result<Option<RaftMsg<C, SD>>, Fatal<C>> {
        match C::AsyncRuntime::mpsc_recv_deadline(&mut self.inner, deadline).await {
            Ok(value) => Ok(Some(value)),
            Err(TryRecvError::Empty) => {
                tracing::debug!("all RaftMsg are processed, wait for more");
                Ok(None)
            }
            Err(TryRecvError::Disconnected) => {
                tracing::debug!("rx_api is disconnected, quit");
                Err(Fatal::Stopped)
            }
        }
    }

    /// Non-blocking receive from the inner receiver.
    fn inner_try_recv(&mut self) -> Result<Option<RaftMsg<C, SD>>, Fatal<C>> {
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
    /// - Maximum batch size is reached
    /// - No more messages are available
    /// - Linger timeout expires before the batch is filled
    async fn merge_client_writes(&mut self, msg: &mut RaftMsg<C, SD>) -> Result<(), Fatal<C>> {
        debug_assert!(self.buffered.is_none());

        let (batch_payloads, batch_responders, batch_leader) = match msg {
            RaftMsg::ClientWrite {
                payloads,
                responders,
                expected_leader,
                ..
            } => (payloads, responders, expected_leader),
            _ => return Ok(()),
        };

        let deadline = C::now().add(self.linger);
        for _ in 1..self.capacity {
            let next = self.inner_recv_timeout_at(deadline).await?;

            let Some(next) = next else {
                break;
            };

            // Can only merge ClientWrite with same expected_leader
            let mergeable = matches!(
                &next,
                RaftMsg::ClientWrite { expected_leader, .. } if expected_leader == batch_leader
            );

            if !mergeable {
                self.buffered = Some(next);
                break;
            }

            match next {
                RaftMsg::ClientWrite {
                    payloads, responders, ..
                } => {
                    batch_payloads.extend(payloads);
                    batch_responders.extend(responders);
                }
                _ => unreachable!(),
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rt::Instant;

    use super::*;
    use crate::async_runtime::MpscSender;
    use crate::batch::Batch;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::entry::EntryPayload;
    use crate::type_config::TypeConfigExt;
    use crate::type_config::alias::BatchOf;
    use crate::type_config::alias::CommittedLeaderIdOf;
    use crate::type_config::alias::EntryPayloadOf;

    type C = UTConfig<()>;

    fn committed_leader_id(term: u64, node_id: u64) -> CommittedLeaderIdOf<C> {
        *log_id(term, node_id, 0).committed_leader_id()
    }

    fn default_msg_receiver(receiver: MpscReceiverOf<C, RaftMsg<C>>) -> BatchRaftMsgReceiver<C> {
        BatchRaftMsgReceiver::new(receiver, 4096, Duration::ZERO)
    }

    fn client_write(data: u64, leader: Option<CommittedLeaderIdOf<C>>) -> RaftMsg<C> {
        RaftMsg::ClientWrite {
            payloads: Batch::of([EntryPayload::Normal(data)]),
            responders: Batch::of([None]),
            expected_leader: leader,
            #[cfg(feature = "runtime-stats")]
            proposed_at: C::now(),
        }
    }

    fn extract_payload_data(payloads: &BatchOf<C, EntryPayloadOf<C>>) -> Vec<u64> {
        payloads
            .as_ref()
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
            let mut receiver: BatchRaftMsgReceiver<C> = default_msg_receiver(rx);

            let leader = Some(committed_leader_id(1, 1));
            tx.send(client_write(1, leader)).await.unwrap();
            tx.send(client_write(2, leader)).await.unwrap();
            tx.send(client_write(3, leader)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();
            let msg = receiver.try_recv().await.unwrap().unwrap();

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
            let mut receiver: BatchRaftMsgReceiver<C> = default_msg_receiver(rx);

            let leader1 = Some(committed_leader_id(1, 1));
            let leader2 = Some(committed_leader_id(2, 1));
            tx.send(client_write(1, leader1)).await.unwrap();
            tx.send(client_write(2, leader2)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();

            // First message should not be merged with second
            let msg1 = receiver.try_recv().await.unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg1
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![1]);
            assert_eq!(responders.len(), payloads.len());

            // Second message should be buffered and returned separately
            let msg2 = receiver.try_recv().await.unwrap().unwrap();
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
            let mut receiver: BatchRaftMsgReceiver<C> = default_msg_receiver(rx);

            let leader = Some(committed_leader_id(1, 1));
            tx.send(client_write(1, leader)).await.unwrap();
            tx.send(RaftMsg::WithRaftState { req: Box::new(|_| {}) }).await.unwrap();
            tx.send(client_write(2, leader)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();

            // First ClientWrite should not merge past the WithRaftState
            let msg1 = receiver.try_recv().await.unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg1
            else {
                panic!("expected ClientWrite");
            };
            assert_eq!(extract_payload_data(&payloads), vec![1]);
            assert_eq!(responders.len(), payloads.len());

            // WithRaftState should be returned next
            let msg2 = receiver.try_recv().await.unwrap().unwrap();
            assert!(matches!(msg2, RaftMsg::WithRaftState { .. }));

            // Last ClientWrite should be returned
            let msg3 = receiver.try_recv().await.unwrap().unwrap();
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
            let mut receiver: BatchRaftMsgReceiver<C> = default_msg_receiver(rx);

            let result = receiver.try_recv().await.unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_ensure_buffered_waits_for_message() {
        C::run(async {
            let (tx, rx) = C::mpsc(100);
            let mut receiver: BatchRaftMsgReceiver<C> = default_msg_receiver(rx);

            tx.send(client_write(42, None)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();

            // Message should be in buffer, try_recv should return it
            let msg = receiver.try_recv().await.unwrap().unwrap();
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
            let mut receiver: BatchRaftMsgReceiver<C> = default_msg_receiver(rx);

            let leader = Some(committed_leader_id(1, 1));
            tx.send(client_write(1, None)).await.unwrap();
            tx.send(client_write(2, leader)).await.unwrap();

            receiver.ensure_buffered().await.unwrap();
            // try_recv will buffer the second message (different leader)
            let _msg1 = receiver.try_recv().await.unwrap().unwrap();

            // ensure_buffered should return immediately since msg2 is buffered
            receiver.ensure_buffered().await.unwrap();

            let msg2 = receiver.try_recv().await.unwrap().unwrap();
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
    fn test_try_recv_batch_up_to_max_capacity() {
        const BATCH_CAPACITY: u64 = 64;
        const NUM_BATCHES: u64 = 2;
        const NUM_CLIENT_WRITES: u64 = BATCH_CAPACITY * NUM_BATCHES;

        C::run(async {
            let (tx, rx) = C::mpsc(NUM_CLIENT_WRITES as usize);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx, BATCH_CAPACITY, Duration::ZERO);

            let leader = Some(committed_leader_id(1, 1));
            for index in 0..NUM_CLIENT_WRITES {
                tx.send(client_write(index, leader)).await.unwrap();
            }

            receiver.ensure_buffered().await.unwrap();

            for _ in 0..NUM_BATCHES {
                let msg = receiver.try_recv().await.unwrap().unwrap();
                let RaftMsg::ClientWrite {
                    payloads, responders, ..
                } = msg
                else {
                    panic!("expected ClientWrite");
                };
                assert_eq!(payloads.len(), BATCH_CAPACITY as usize);
                assert_eq!(responders.len(), BATCH_CAPACITY as usize);
            }
        });
    }

    #[test]
    fn test_try_recv_flush_batch_after_linger() {
        const BATCH_CAPACITY: u64 = 100;
        const LINGER: Duration = Duration::from_secs(1);

        C::run(async {
            let (tx, rx) = C::mpsc(BATCH_CAPACITY as usize);
            let mut receiver: BatchRaftMsgReceiver<C> = BatchRaftMsgReceiver::new(rx, BATCH_CAPACITY, LINGER);

            let now = C::now();

            let leader = Some(committed_leader_id(1, 1));
            tx.send(client_write(0, leader)).await.unwrap();

            let msg = receiver.try_recv().await.unwrap().unwrap();
            let RaftMsg::ClientWrite {
                payloads, responders, ..
            } = msg
            else {
                panic!("expected ClientWrite");
            };

            assert_eq!(1, payloads.len());
            assert_eq!(1, responders.len());
            assert_eq!(Duration::ZERO, LINGER.saturating_sub(now.elapsed()))
        });
    }
}
