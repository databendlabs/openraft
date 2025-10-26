use std::collections::VecDeque;
use std::fmt;

use crate::LogId;
use crate::RaftTypeConfig;
use crate::engine::Respond;
use crate::engine::respond_command::PendingRespond;
use crate::raft_state::IOId;
use crate::raft_state::io_state::IOState;

/// Queues of pending responds waiting for IO conditions to be satisfied.
#[derive(Debug, Default)]
pub(crate) struct PendingResponds<C>
where C: RaftTypeConfig
{
    /// Responds waiting for log IO to be flushed to storage.
    pub(crate) on_log_io: VecDeque<PendingRespond<C, IOId<C>>>,

    /// Responds waiting for log entries to be flushed to storage.
    pub(crate) on_log_flush: VecDeque<PendingRespond<C, LogId<C>>>,

    /// Responds waiting for log entries to be applied to state machine.
    pub(crate) on_apply: VecDeque<PendingRespond<C, LogId<C>>>,

    /// Responds waiting for snapshot to be built.
    pub(crate) on_snapshot: VecDeque<PendingRespond<C, LogId<C>>>,
}

impl<C> PendingResponds<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            on_log_io: VecDeque::with_capacity(capacity),
            on_log_flush: VecDeque::with_capacity(capacity),
            on_apply: VecDeque::with_capacity(capacity),
            on_snapshot: VecDeque::with_capacity(capacity),
        }
    }

    /// Drain all satisfied responds based on the current IO state.
    ///
    /// Returns an iterator that yields all responds whose conditions are met by the provided
    /// IO state. The responds are removed from their respective queues as they are yielded.
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
        return queue.pop_front().map(|p| p.into_respond());
    }
    None
}

/// Iterator that drains satisfied responds from pending queues.
///
/// Iterates through all phases (LogIO, LogFlush, Apply, Snapshot) and yields responds
/// whose waiting conditions are satisfied by the current IO state.
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
mod tests {
    use validit::Valid;

    use super::*;
    use crate::AsyncRuntime;
    use crate::LogId;
    use crate::Vote;
    use crate::engine::Respond;
    use crate::engine::respond_command::PendingRespond;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::impls::TokioRuntime;
    use crate::raft::VoteResponse;
    use crate::raft_state::IOId;
    use crate::raft_state::io_state::IOState;
    use crate::raft_state::io_state::io_progress::IOProgress;
    use crate::type_config::async_runtime::oneshot::Oneshot;

    type TestIOId = IOId<UTConfig>;
    type TestLogId = LogId<UTConfig>;

    fn new_respond() -> Respond<UTConfig> {
        let (tx, _rx) = <TokioRuntime as AsyncRuntime>::Oneshot::channel();
        Respond::new(VoteResponse::new(Vote::new(1, 1), None, false), tx)
    }

    fn new_io_state(
        log_io: Option<TestIOId>,
        applied: Option<TestLogId>,
        snapshot: Option<TestLogId>,
    ) -> IOState<UTConfig> {
        let vote = Vote::new(1, 1);
        let mut io_state = IOState::new("xx", &vote, applied, snapshot, None, false);
        if let Some(id) = log_io {
            io_state.log_progress = Valid::new(IOProgress::new_synchronized(Some(id), "xx", "log", false));
        }
        io_state
    }

    #[test]
    fn test_pop_if_satisfied() {
        let mut queue = VecDeque::new();
        queue.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));
        queue.push_back(PendingRespond::new(log_id(1, 1, 5), new_respond()));

        assert!(pop_if_satisfied(&mut queue, &log_id(1, 1, 2)).is_none());
        assert_eq!(queue.len(), 2);

        assert!(pop_if_satisfied(&mut queue, &log_id(1, 1, 3)).is_some());
        assert_eq!(queue.len(), 1);

        assert!(pop_if_satisfied(&mut queue, &log_id(1, 1, 10)).is_some());
        assert_eq!(queue.len(), 0);

        assert!(pop_if_satisfied(&mut queue, &log_id(1, 1, 10)).is_none());
    }

    #[test]
    fn test_drain_satisfied_empty_queues() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        let io_state = new_io_state(None, None, None);

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 0);
    }

    #[test]
    fn test_drain_satisfied_all_unsatisfied() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 5), new_respond()));
        pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 5), new_respond()));

        let io_state = new_io_state(None, None, None);

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 0);
        assert_eq!(pending.on_log_flush.len(), 1);
        assert_eq!(pending.on_apply.len(), 1);
    }

    #[test]
    fn test_drain_satisfied_log_io() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        let io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 3)));
        pending.on_log_io.push_back(PendingRespond::new(io_id, new_respond()));

        let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
        let io_state = new_io_state(Some(flushed_io_id), None, None);

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, DrainPhase::LogIO);
        assert_eq!(pending.on_log_io.len(), 0);
    }

    #[test]
    fn test_drain_satisfied_log_flush() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));

        let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
        let io_state = new_io_state(Some(flushed_io_id), None, None);

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, DrainPhase::LogFlush);
        assert_eq!(pending.on_log_flush.len(), 0);
    }

    #[test]
    fn test_drain_satisfied_apply() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));

        let io_state = new_io_state(None, Some(log_id(1, 1, 5)), None);

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, DrainPhase::Apply);
        assert_eq!(pending.on_apply.len(), 0);
    }

    #[test]
    fn test_drain_satisfied_snapshot() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));

        let io_state = new_io_state(None, None, Some(log_id(1, 1, 5)));

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, DrainPhase::Snapshot);
        assert_eq!(pending.on_snapshot.len(), 0);
    }

    #[test]
    fn test_drain_satisfied_multiple_phases() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        let io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 3)));
        pending.on_log_io.push_back(PendingRespond::new(io_id, new_respond()));
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));
        pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));
        pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));

        let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
        let io_state = new_io_state(Some(flushed_io_id), Some(log_id(1, 1, 5)), Some(log_id(1, 1, 5)));

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 4);
        assert_eq!(collected[0].0, DrainPhase::LogIO);
        assert_eq!(collected[1].0, DrainPhase::LogFlush);
        assert_eq!(collected[2].0, DrainPhase::Apply);
        assert_eq!(collected[3].0, DrainPhase::Snapshot);
    }

    #[test]
    fn test_drain_satisfied_partial_satisfaction() {
        let mut pending = PendingResponds::<UTConfig>::new(10);
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 5), new_respond()));
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 7), new_respond()));

        let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
        let io_state = new_io_state(Some(flushed_io_id), None, None);

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(pending.on_log_flush.len(), 1);
    }

    #[test]
    fn test_drain_satisfied_multiple_items_per_phase() {
        let mut pending = PendingResponds::<UTConfig>::new(10);

        let io_id1 = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 2)));
        let io_id2 = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 3)));
        let io_id3 = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 7)));
        pending.on_log_io.push_back(PendingRespond::new(io_id1, new_respond()));
        pending.on_log_io.push_back(PendingRespond::new(io_id2, new_respond()));
        pending.on_log_io.push_back(PendingRespond::new(io_id3, new_respond()));

        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 2), new_respond()));
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 4), new_respond()));
        pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 8), new_respond()));

        pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 2), new_respond()));
        pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 4), new_respond()));
        pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 9), new_respond()));

        pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 3), new_respond()));
        pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 10), new_respond()));

        let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
        let io_state = new_io_state(Some(flushed_io_id), Some(log_id(1, 1, 5)), Some(log_id(1, 1, 5)));

        let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
        assert_eq!(collected.len(), 8);

        assert_eq!(collected[0].0, DrainPhase::LogIO);
        assert_eq!(collected[1].0, DrainPhase::LogIO);
        assert_eq!(collected[2].0, DrainPhase::LogFlush);
        assert_eq!(collected[3].0, DrainPhase::LogFlush);
        assert_eq!(collected[4].0, DrainPhase::LogFlush);
        assert_eq!(collected[5].0, DrainPhase::Apply);
        assert_eq!(collected[6].0, DrainPhase::Apply);
        assert_eq!(collected[7].0, DrainPhase::Snapshot);

        assert_eq!(pending.on_log_io.len(), 1);
        assert_eq!(pending.on_log_flush.len(), 1);
        assert_eq!(pending.on_apply.len(), 1);
        assert_eq!(pending.on_snapshot.len(), 1);
    }

    #[test]
    fn test_drain_phase_display() {
        assert_eq!(format!("{}", DrainPhase::LogIO), "log_io");
        assert_eq!(format!("{}", DrainPhase::LogFlush), "log_flush");
        assert_eq!(format!("{}", DrainPhase::Apply), "apply");
        assert_eq!(format!("{}", DrainPhase::Snapshot), "snapshot");
        assert_eq!(format!("{}", DrainPhase::Done), "done");
    }
}
