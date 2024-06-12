//! Callbacks used by Storage API

use std::io;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

use tokio::sync::mpsc;

use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::Vote;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::StorageIOError;

struct LogEvent<R> (R, u64);

impl<R> Ord for LogEvent<R> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.1.cmp(&other.1).reverse()
    }
}

impl<R> PartialOrd for LogEvent<R> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.1.cmp(&other.1).reverse())
    }
}

impl<R> PartialEq for LogEvent<R> {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1
    }
}

impl<R> Eq for LogEvent<R> {}

pub struct LogEventSender<R> {
    event_seq: u64,
    sender: Option<mpsc::UnboundedSender<LogEvent<R>>>,
}

impl<R> Drop for LogEventSender<R> {
    fn drop(&mut self) {
        if self.sender.take().is_some() {
            panic!("dropped without sending event");
        }
    }
}

impl<R> LogEventSender<R> {
    fn send(mut self, evt: R) -> Result<(), mpsc::error::SendError<R>> {
        let sender = self.sender.take().unwrap();
        sender.send(LogEvent(evt, self.event_seq))
            .map_err(|err| mpsc::error::SendError(err.0.0))
    }
}

pub struct LogEventChannel<R> {
    tx_log_events: mpsc::UnboundedSender<LogEvent<R>>,
    rx_log_events: mpsc::UnboundedReceiver<LogEvent<R>>,
    send_seq: u64,
    recv_seq: u64,
    recv_heap: BinaryHeap<LogEvent<R>>,
}

impl<R> LogEventChannel<R> {
    pub fn new() -> Self {
        let (tx_log_events, rx_log_events) = mpsc::unbounded_channel();
        Self {
            tx_log_events,
            rx_log_events,
            send_seq: 0,
            recv_seq: 0,
            recv_heap: BinaryHeap::new(),
        }
    }

    pub fn try_recv(&mut self) -> Option<R> {
        if let Some(ev) = self.recv_heap.peek() {
            if ev.1 == self.recv_seq + 1 {
                self.recv_seq += 1;
                return Some(self.recv_heap.pop().unwrap().0);
            }
        }
        while let Ok(ev) = self.rx_log_events.try_recv() {
            if ev.1 == self.recv_seq + 1 {
                self.recv_seq += 1;
                return Some(ev.0)
            }
            self.recv_heap.push(ev);
        }
        None
    }

    pub async fn wait_next(&mut self) -> Result<(), mpsc::error::TryRecvError> {
        if let Some(ev) = self.recv_heap.peek() {
            if ev.1 == self.recv_seq + 1 {
                return Ok(());
            }
        }
        while let Some(ev) = self.rx_log_events.recv().await {
            let event_seq = ev.1;
            self.recv_heap.push(ev);
            if event_seq == self.recv_seq + 1 {
                return Ok(());
            }
        }
        Err(mpsc::error::TryRecvError::Disconnected)
    }

    fn new_sender(&mut self) ->  LogEventSender<R> {
        let sender = Some(self.tx_log_events.clone());
        self.send_seq += 1;
        let event_seq = self.send_seq;
        LogEventSender { event_seq, sender }
    }
}

#[derive(Debug)]
pub(crate) enum LogFlushKind<C>
where C: RaftTypeConfig
{
    Append(LogIOId<C::NodeId>),
    SaveVote(Vote<C::NodeId>),
}

/// A oneshot callback for completion of log io operation.
pub struct LogFlushed<C>
where C: RaftTypeConfig
{
    io_kind: LogFlushKind<C>,
    sender: LogEventSender<Result<LogFlushKind<C>, io::Error>>,
}

impl<C> LogFlushed<C>
where C: RaftTypeConfig
{
    pub(crate) fn with_append(
        log_io_id: LogIOId<C::NodeId>,
        event_chan: &mut LogEventChannel<Result<LogFlushKind<C>, io::Error>>,
    ) -> Self {
        Self {
            io_kind: LogFlushKind::Append(log_io_id),
            sender: event_chan.new_sender()
        }
    }

    pub(crate) fn with_save_vote(
        vote: Vote<C::NodeId>,
        event_chan: &mut LogEventChannel<Result<LogFlushKind<C>, io::Error>>,
    ) -> Self {
        Self {
            io_kind: LogFlushKind::SaveVote(vote),
            sender: event_chan.new_sender()
        }
    }

    /// Report log io completion event.
    ///
    /// It will be called when the log is successfully appended to the storage or an error occurs.
    pub fn log_io_completed(self, result: Result<(), io::Error>) {
        let res = if let Err(e) = result {
            tracing::error!("LogFlush error: {}, io_kind {:?}", e, self.io_kind);
            self.sender.send(Err(e))
        } else {
            self.sender.send(Ok(self.io_kind))
        };

        if let Err(e) = res {
            tracing::error!("failed to send log io completion event: {:?}", e);
        }
    }
}

/// A oneshot callback for completion of applying logs to state machine.
pub struct LogApplied<C>
where C: RaftTypeConfig
{
    last_log_id: LogId<C::NodeId>,
    sender: LogEventSender<Result<(LogId<C::NodeId>, Vec<C::R>), StorageIOError<C::NodeId>>>,
}

impl<C> LogApplied<C>
where C: RaftTypeConfig
{
    #[allow(dead_code)]
    pub(crate) fn new(
        last_log_id: LogId<C::NodeId>,
        event_chan: &mut LogEventChannel<Result<(LogId<C::NodeId>, Vec<C::R>), StorageIOError<C::NodeId>>>,
    ) -> Self {
        Self {
            last_log_id,
            sender: event_chan.new_sender()
        }
    }

    /// Report apply io completion event.
    ///
    /// It will be called when the log is successfully applied to the state machine or an error
    /// occurs.
    pub fn completed(self, result: Result<Vec<C::R>, StorageIOError<C::NodeId>>) {
        let res = match result {
            Ok(x) => {
                tracing::debug!("LogApplied upto {}", self.last_log_id);
                let resp = (self.last_log_id, x);
                self.sender.send(Ok(resp))
            }
            Err(e) => {
                tracing::error!("LogApplied error: {}, while applying upto {}", e, self.last_log_id);
                self.sender.send(Err(e))
            }
        };

        if let Err(_e) = res {
            tracing::error!("failed to send apply complete event, last_log_id: {}", self.last_log_id);
        }
    }
}
