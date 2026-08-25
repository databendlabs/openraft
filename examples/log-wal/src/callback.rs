//! The callback `raft_log` invokes when a flush finishes.

use std::io;

use openraft::RaftTypeConfig;
use openraft::storage::IOFlushed;
use tokio::sync::oneshot;

/// Delivers the result of a `raft_log` flush to whoever is waiting for it.
///
/// `raft_log::RaftLogWriter::flush` hands the write to a background worker and
/// returns at once. The worker calls [`raft_log::Callback::send`] after the
/// data reaches disk, and this type routes that result to the right waiter.
pub enum Callback<C>
where C: RaftTypeConfig
{
    /// Reports the result to openraft, which tracks it as flushed IO.
    IOFlushed(IOFlushed<C>),

    /// Reports the result to a caller that awaits the flush before returning.
    Oneshot(oneshot::Sender<Result<(), io::Error>>),
}

impl<C> raft_log::Callback for Callback<C>
where C: RaftTypeConfig
{
    fn send(self, res: Result<(), io::Error>) {
        match self {
            Self::IOFlushed(io_flushed) => io_flushed.io_completed(res),
            Self::Oneshot(tx) => {
                let send_res = tx.send(res);
                if send_res.is_err() {
                    tracing::warn!("log-wal: the caller waiting for this flush result is gone");
                }
            }
        }
    }
}
