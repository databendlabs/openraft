//! tick emitter emits a `RaftMsg::Tick` event at a certain interval.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::sleep_until;
use tokio::time::Instant;
use tracing::Level;
use tracing::Span;
use tracing_futures::Instrument;

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

pub(crate) struct Tick<C, N, S>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    S: RaftStorage<C>,
{
    interval: Duration,

    tx: mpsc::UnboundedSender<(RaftMsg<C, N, S>, Span)>,
}

impl<C, N, S> Tick<C, N, S>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    S: RaftStorage<C>,
{
    pub(crate) fn spawn(interval: Duration, tx: mpsc::UnboundedSender<(RaftMsg<C, N, S>, Span)>) -> JoinHandle<()> {
        let t = Tick { interval, tx };

        tokio::spawn(
            async move {
                let mut i = 0;
                loop {
                    i += 1;

                    let at = Instant::now() + t.interval;
                    sleep_until(at).await;

                    let send_res = t.tx.send((RaftMsg::Tick { i }, tracing::span!(Level::DEBUG, "tick")));
                    if let Err(_e) = send_res {
                        tracing::info!("Tick fails to send, receiving end quit.");
                    } else {
                        tracing::debug!("Tick sent: {}", i)
                    }
                }
            }
            .instrument(tracing::span!(parent: &Span::current(), Level::DEBUG, "tick")),
        )
    }
}
