use openraft_macros::since;

use crate::OptionalSend;
use crate::mpsc::MpscReceiver;
use crate::mpsc::TryRecvError;

/// An MPSC receiver that buffers one item for inspection before it is received.
///
/// Peeking removes an item from the inner channel and frees one slot in its buffer.
/// The item stays in this wrapper until it is received.
#[since(
    version = "0.10.1",
    change = "rename `peek()` to `try_peek()` and add async `peek()`"
)]
#[since(version = "0.10.0", change = "add peekable MPSC receiver")]
pub struct PeekableReceiver<R, T>
where
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    inner: R,
    buffered: Option<T>,
}

impl<R, T> PeekableReceiver<R, T>
where
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    /// Wraps an MPSC receiver with an empty peek buffer.
    #[since(version = "0.10.0")]
    pub fn new(inner: R) -> Self {
        Self { inner, buffered: None }
    }

    /// Inspects the next item without consuming it, waiting for one if none is buffered.
    ///
    /// Repeated calls return the same item until it is received.
    /// Returns `None` if the channel is closed and no item remains, like [`MpscReceiver::recv`].
    /// Cancelling the returned future is as safe as cancelling `recv` on the inner receiver.
    #[since(version = "0.10.1")]
    pub async fn peek(&mut self) -> Option<&T> {
        if self.buffered.is_none() {
            self.buffered = self.inner.recv().await;
        }

        self.buffered.as_ref()
    }

    /// Inspects the next item without consuming it or waiting for an item to arrive.
    ///
    /// Repeated calls return the same item until it is received.
    /// Returns the same errors as [`MpscReceiver::try_recv`] when no item is buffered.
    #[since(version = "0.10.1", change = "renamed from `peek()`")]
    #[since(version = "0.10.0")]
    pub fn try_peek(&mut self) -> Result<&T, TryRecvError> {
        if self.buffered.is_none() {
            let item = self.inner.try_recv()?;
            self.buffered = Some(item);
        }

        let item = self.buffered.as_ref().unwrap();
        Ok(item)
    }
}

impl<R, T> MpscReceiver<T> for PeekableReceiver<R, T>
where
    R: MpscReceiver<T>,
    T: OptionalSend,
{
    async fn recv(&mut self) -> Option<T> {
        if let Some(item) = self.buffered.take() {
            return Some(item);
        }

        self.inner.recv().await
    }

    fn try_recv(&mut self) -> Result<T, TryRecvError> {
        if let Some(item) = self.buffered.take() {
            return Ok(item);
        }

        self.inner.try_recv()
    }
}
