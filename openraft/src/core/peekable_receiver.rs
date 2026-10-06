use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::TryRecvError;
use crate::type_config::alias::MpscReceiverOf;

/// An MPSC receiver that buffers one item for nonblocking inspection.
pub(crate) struct PeekableReceiver<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend,
{
    inner: MpscReceiverOf<C, T>,
    buffered: Option<T>,
}

impl<C, T> PeekableReceiver<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend,
{
    pub(crate) fn new(inner: MpscReceiverOf<C, T>) -> Self {
        Self { inner, buffered: None }
    }

    /// Inspect the next item without consuming it or waiting for an item to arrive.
    pub(crate) fn peek(&mut self) -> Result<&T, TryRecvError> {
        if self.buffered.is_none() {
            let item = self.inner.try_recv()?;
            self.buffered = Some(item);
        }

        let item = self.buffered.as_ref().unwrap();
        Ok(item)
    }

    pub(crate) async fn recv(&mut self) -> Option<T> {
        if let Some(item) = self.buffered.take() {
            return Some(item);
        }

        self.inner.recv().await
    }

    pub(crate) fn try_recv(&mut self) -> Result<T, TryRecvError> {
        if let Some(item) = self.buffered.take() {
            return Ok(item);
        }

        self.inner.try_recv()
    }
}

#[cfg(test)]
mod peekable_receiver_test;
