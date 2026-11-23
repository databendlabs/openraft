use std::collections::VecDeque;
use std::fmt;

use crate::RaftTypeConfig;
use crate::engine::Respond;
use crate::engine::respond_command::PendingRespond;
use crate::raft_state::IOId;
use crate::raft_state::io_state::IOState;
use crate::type_config::alias::LogIdOf;

/// Queues of pending responds waiting for IO conditions to be satisfied.
#[derive(Debug, Default)]
pub(crate) struct PendingResponds<C>
where C: RaftTypeConfig
{
    /// Responds waiting for log IO to be flushed to storage.
    pub(crate) on_log_io: VecDeque<PendingRespond<C, IOId<C>>>,

    /// Responds waiting for log entries to be flushed to storage.
    pub(crate) on_log_flush: VecDeque<PendingRespond<C, LogIdOf<C>>>,

    /// Responds waiting for log entries to be applied to state machine.
    pub(crate) on_apply: VecDeque<PendingRespond<C, LogIdOf<C>>>,

    /// Responds waiting for snapshot to be built.
    pub(crate) on_snapshot: VecDeque<PendingRespond<C, LogIdOf<C>>>,
}

impl<C> PendingResponds<C>
where C: RaftTypeConfig
{
    /// Drain the satisfied responds at the front of each queue, based on the current IO state.
    ///
    /// Returns an iterator that yields, queue by queue, the responds at the front whose conditions
    /// the IO state satisfies, and removes each one as it yields it. A queue keeps its responds in
    /// the order their requests arrived, and its drain stops at the first respond that is not
    /// satisfied: a satisfied respond behind it waits until every respond before it is satisfied.
    ///
    /// Steady-state replication queues conditions in increasing order: the leader sends appends in
    /// log order, and a heartbeat carries a matching log ID that the follower has already flushed,
    /// so its respond is sent without being queued. A respond waits behind another one only in a
    /// transition, for example the retry of an append whose RPC timed out, which can end before an
    /// append queued before it; its respond is sent once that append is flushed.
    pub(crate) fn drain_satisfied<'a>(&'a mut self, io_state: &'a IOState<C>) -> DrainSatisfied<'a, C> {
        DrainSatisfied {
            pending_responds: self,
            io_state,
            phase: DrainPhase::LogIO,
        }
    }
}

/// Pop the first respond from the queue if the actual value satisfies the pending condition.
pub(crate) fn pop_if_satisfied<C, V>(queue: &mut VecDeque<PendingRespond<C, V>>, actual: &V) -> Option<Respond<C>>
where
    C: RaftTypeConfig,
    V: PartialOrd,
{
    let front_item = queue.front()?;

    if actual >= front_item.wait_for() {
        let respond = queue.pop_front().map(|p| p.into_respond());
        // Reset the head so a queue that has grown does not walk its whole buffer.
        if queue.is_empty() {
            queue.clear();
        }
        return respond;
    }
    None
}

/// Iterator that drains satisfied responds from pending queues.
///
/// Iterates through all phases (LogIO, LogFlush, Apply, Snapshot) and yields the responds at the
/// front of each queue whose waiting conditions are satisfied by the current IO state; see
/// [`PendingResponds::drain_satisfied()`].
pub(crate) struct DrainSatisfied<'a, C>
where C: RaftTypeConfig
{
    pending_responds: &'a mut PendingResponds<C>,
    io_state: &'a IOState<C>,
    phase: DrainPhase,
}

/// Phases for draining satisfied responds from pending queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrainPhase {
    /// Draining responds waiting for log IO to be flushed.
    LogIO,
    /// Draining responds waiting for log entries to be flushed.
    LogFlush,
    /// Draining responds waiting for log entries to be applied.
    Apply,
    /// Draining responds waiting for snapshot to be built.
    Snapshot,
    /// All phases completed.
    Done,
}

impl fmt::Display for DrainPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrainPhase::LogIO => write!(f, "log_io"),
            DrainPhase::LogFlush => write!(f, "log_flush"),
            DrainPhase::Apply => write!(f, "apply"),
            DrainPhase::Snapshot => write!(f, "snapshot"),
            DrainPhase::Done => write!(f, "done"),
        }
    }
}

impl<'a, C> Iterator for DrainSatisfied<'a, C>
where C: RaftTypeConfig
{
    type Item = (DrainPhase, Respond<C>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.phase {
                DrainPhase::LogIO => {
                    if let Some(flushed) = self.io_state.log_progress.flushed()
                        && let Some(respond) = pop_if_satisfied(&mut self.pending_responds.on_log_io, flushed)
                    {
                        return Some((self.phase, respond));
                    }
                    self.phase = DrainPhase::LogFlush;
                }
                DrainPhase::LogFlush => {
                    if let Some(flushed) = self.io_state.log_progress.flushed()
                        && let Some(log_id) = flushed.last_log_id()
                        && let Some(respond) = pop_if_satisfied(&mut self.pending_responds.on_log_flush, log_id)
                    {
                        return Some((self.phase, respond));
                    }
                    self.phase = DrainPhase::Apply;
                }
                DrainPhase::Apply => {
                    if let Some(applied) = self.io_state.apply_progress.flushed()
                        && let Some(respond) = pop_if_satisfied(&mut self.pending_responds.on_apply, applied)
                    {
                        return Some((self.phase, respond));
                    }
                    self.phase = DrainPhase::Snapshot;
                }
                DrainPhase::Snapshot => {
                    if let Some(snapshot) = self.io_state.snapshot.flushed()
                        && let Some(respond) = pop_if_satisfied(&mut self.pending_responds.on_snapshot, snapshot)
                    {
                        return Some((self.phase, respond));
                    }
                    self.phase = DrainPhase::Done;
                }
                DrainPhase::Done => {
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod pending_responds_test;
