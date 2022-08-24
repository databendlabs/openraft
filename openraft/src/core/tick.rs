//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::sleep_until;
use tokio::time::Instant;

use crate::raft::RaftMsg;
use crate::NodeId;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::Vote;

/// An instant time point bound to a vote.
///
/// If the vote on a node changes, the timeout belonging to a previous vote becomes invalid.
/// See: https://datafuselabs.github.io/openraft/vote.html
#[derive(Debug)]
pub(crate) struct VoteWiseTime<NID: NodeId> {
    pub(crate) vote: Vote<NID>,
    pub(crate) time: Instant,
}

impl<NID: NodeId> VoteWiseTime<NID> {
    pub(crate) fn new(vote: Vote<NID>, time: Instant) -> Self {
        Self { vote, time }
    }

    /// Return the time if vote does not change since it is set.
    pub(crate) fn get_time(&self, current_vote: &Vote<NID>) -> Option<Instant> {
        debug_assert!(&self.vote <= current_vote);

        if &self.vote == current_vote {
            Some(self.time)
        } else {
            None
        }
    }
}

/// Emit RaftMsg::Tick event at regular `interval`.
pub(crate) struct Tick<C, N, S>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    S: RaftStorage<C>,
{
    interval: Duration,

    tx: mpsc::UnboundedSender<RaftMsg<C, N, S>>,

    /// Emit event or not
    running: Arc<AtomicBool>,
}

pub(crate) struct TickHandle {
    running: Arc<AtomicBool>,
}

impl<C, N, S> Tick<C, N, S>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    S: RaftStorage<C>,
{
    pub(crate) fn new(interval: Duration, tx: mpsc::UnboundedSender<RaftMsg<C, N, S>>, enabled: bool) -> Self {
        Tick {
            interval,
            running: Arc::new(AtomicBool::from(enabled)),
            tx,
        }
    }

    pub(crate) async fn tick_loop(self) {
        let mut i = 0;
        loop {
            i += 1;

            let at = Instant::now() + self.interval;
            sleep_until(at).await;

            if !self.running.load(Ordering::Relaxed) {
                i -= 1;
                continue;
            }

            let send_res = self.tx.send(RaftMsg::Tick { i });
            if let Err(e) = send_res {
                tracing::info!("Tick fails to send, receiving end quit: {e}");
            } else {
                tracing::debug!("Tick sent: {}", i)
            }
        }
    }

    /// Return a handle to control the ticker.
    pub(crate) fn get_handle(&self) -> TickHandle {
        TickHandle {
            running: self.running.clone(),
        }
    }
}

impl TickHandle {
    pub(crate) fn enable(&self, enabled: bool) {
        self.running.store(enabled, Ordering::Relaxed);
    }
}
