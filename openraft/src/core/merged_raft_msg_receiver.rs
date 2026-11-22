//! Batching receiver for RaftMsg that merges consecutive ClientWrite messages.
//!
//! This module provides [`BatchRaftMsgReceiver`], a wrapper around an mpsc receiver
//! that automatically batches consecutive `RaftMsg::ClientWrite` messages with
//! mergeable [`WriteOption`]s into a single message, improving throughput by reducing
//! per-message overhead.
//!
//! [`WriteOption`]: crate::raft::WriteOption

use std::ops::Add;
use std::time::Duration;

use rt::AsyncRuntime;

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
/// whose `write_option`s are mergeable into a single message. This batching
/// reduces per-message processing overhead and improves write throughput.
///
/// # Batching Rules
///
/// - Only `RaftMsg::ClientWrite` messages are batched
/// - A message is merged only if [`WriteOption::merge()`] merges its `write_option` with the
///   batch's; the batch is then proposed with the merged option
/// - A non-`ClientWrite` message, or a `ClientWrite` that does not merge, stops the batching and is
///   buffered for the next receive
/// - A batch holds at most `capacity` input messages, counting the first one. `Raft::new` sets it
///   from `Config::api_batch_capacity`, 4096 by default. One message can carry several payloads,
///   and a capacity of 0 or 1 merges no message.
/// - A batch waits for more messages until one deadline, `linger` after its first message; see
///   [`try_recv()`](Self::try_recv)
///
/// # Usage Pattern
///
/// ```ignore
/// // Wait for at least one message
/// receiver.ensure_buffered().await?;
///
/// // Process all available messages (with batching)
/// while let Some(msg) = receiver.try_recv().await? {
///     handle_msg(msg);
/// }
/// ```
///
/// [`WriteOption::merge()`]: crate::raft::WriteOption::merge
pub(crate) struct BatchRaftMsgReceiver<C>
where C: RaftTypeConfig
{
    /// A message that was received but not yet returned to the caller.
    ///
    /// This is used when:
    /// - A non-mergeable message is encountered during batching
    /// - A `ClientWrite` with a non-mergeable `write_option` is encountered
    buffered: Option<RaftMsg<C>>,

    /// The most input messages a batch holds, counting the first one.
    capacity: u64,

    /// How long after its first message a batch waits for more, unless it fills up first.
    linger: Duration,

    /// The underlying mpsc receiver.
    inner: MpscReceiverOf<C, RaftMsg<C>>,
}

impl<C> BatchRaftMsgReceiver<C>
where C: RaftTypeConfig
{
    /// Creates a new batching receiver given:
    /// - mpsc receiver channel
    /// - capacity
    /// - linger
    pub(crate) fn new(receiver: MpscReceiverOf<C, RaftMsg<C>>, capacity: u64, linger: Duration) -> Self {
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

    /// Receives a message, and merges the mergeable `ClientWrite` messages that follow it.
    ///
    /// Returns `Ok(None)` at once if no message is buffered or queued, and
    /// `Err(Fatal::Stopped)` if the sender is dropped.
    ///
    /// If the message is a `ClientWrite`, this call can wait: it merges the queued messages and,
    /// with a positive linger, those that arrive before the batch's deadline, `linger` after the
    /// first message. A full batch or a message that cannot be merged returns the batch before
    /// the deadline. With a linger of 0, it merges only the messages already queued.
    pub(crate) async fn try_recv(&mut self) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
        let msg = self.buffered_try_recv()?;

        let Some(mut msg) = msg else {
            return Ok(None);
        };

        self.merge_client_writes(&mut msg).await?;

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
    ) -> Result<Option<RaftMsg<C>>, Fatal<C>> {
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

    /// Merges consecutive `ClientWrite` messages whose `write_option`s merge.
    ///
    /// Reads additional messages from the inner receiver and merges them into `msg`
    /// if they are `ClientWrite` whose `write_option` merges with `msg`'s, and replaces `msg`'s
    /// `write_option` with the merged one. Stops merging when:
    /// - A non-`ClientWrite` message is encountered (buffered for next recv)
    /// - A `ClientWrite` whose `write_option` does not merge is found (buffered for next recv)
    /// - The batch holds `capacity` messages
    /// - No message is queued after the deadline, `linger` after the first message. Until the
    ///   deadline it waits for one, and a message already queued is merged even after it, because
    ///   each receive checks the queue first.
    async fn merge_client_writes(&mut self, msg: &mut RaftMsg<C>) -> Result<(), Fatal<C>> {
        debug_assert!(self.buffered.is_none());

        let (batch_payloads, batch_responders, batch_option) = match msg {
            RaftMsg::ClientWrite {
                payloads,
                responders,
                write_option,
                ..
            } => (payloads, responders, write_option),
            _ => return Ok(()),
        };

        let deadline = C::now().add(self.linger);
        for _ in 1..self.capacity {
            let next = self.inner_recv_timeout_at(deadline).await?;

            let Some(next) = next else {
                break;
            };

            let merged_option = match &next {
                RaftMsg::ClientWrite { write_option, .. } => batch_option.merge(write_option),
                _ => None,
            };

            let Some(merged_option) = merged_option else {
                self.buffered = Some(next);
                break;
            };

            match next {
                RaftMsg::ClientWrite {
                    payloads, responders, ..
                } => {
                    *batch_option = merged_option;
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
mod merged_raft_msg_receiver_test;
