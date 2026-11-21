use std::fmt;

/// Error returned by [`WatchSender::send()`](crate::WatchSender::send) when every receiver has
/// been dropped.
///
/// The field holds the value that was not sent.
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendError").finish_non_exhaustive()
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "watch channel closed")
    }
}

impl<T> std::error::Error for SendError<T> {}

/// Error returned by [`WatchReceiver::changed()`](crate::WatchReceiver::changed), and the waits
/// built on it, once every sender has been dropped and this receiver has seen the final value.
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct RecvError(pub ());

impl fmt::Debug for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecvError").finish_non_exhaustive()
    }
}

impl fmt::Display for RecvError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "watch channel closed")
    }
}

impl std::error::Error for RecvError {}
