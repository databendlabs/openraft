use std::collections::VecDeque;

use futures_util::FutureExt;
use validit::Valid;

use super::DrainPhase;
use super::PendingResponds;
use super::pop_if_satisfied;
use crate::Vote;
use crate::engine::Respond;
use crate::engine::respond_command::PendingRespond;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteResponse;
use crate::raft_state::IOId;
use crate::raft_state::io_state::IOState;
use crate::raft_state::io_state::io_progress::IOProgress;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::OneshotReceiverOf;

type TestIOId = IOId<UTConfig>;
type TestLogId = LogIdOf<UTConfig>;
type ReplyRx = OneshotReceiverOf<UTConfig, VoteResponse<UTConfig>>;

/// The reply tagged `tag`: a vote response whose vote has the term `tag`.
fn reply(tag: u64) -> VoteResponse<UTConfig> {
    VoteResponse::new(Vote::new(tag, 1), None, false)
}

/// A respond that sends `reply(tag)`, and the receiver it sends to.
fn new_respond(tag: u64) -> (Respond<UTConfig>, ReplyRx) {
    let (tx, rx) = UTConfig::<()>::oneshot();
    (Respond::new(reply(tag), tx), rx)
}

/// The reply a respond sends.
fn reply_of(respond: &Respond<UTConfig>) -> VoteResponse<UTConfig> {
    match respond {
        Respond::Vote(sender) => sender.value().clone(),
        other => panic!("unexpected respond: {}", other),
    }
}

/// The phase and the reply of each drained respond, in drain order.
fn replies(drained: &[(DrainPhase, Respond<UTConfig>)]) -> Vec<(DrainPhase, VoteResponse<UTConfig>)> {
    drained.iter().map(|(phase, respond)| (*phase, reply_of(respond))).collect()
}

/// The conditions a queue still waits for, front first.
fn waiting<V>(queue: &VecDeque<PendingRespond<UTConfig, V>>) -> Vec<V>
where V: Clone {
    queue.iter().map(|p| p.wait_for().clone()).collect()
}

fn new_io_state(
    log_io: Option<TestIOId>,
    applied: Option<TestLogId>,
    snapshot: Option<TestLogId>,
) -> IOState<UTConfig> {
    let vote = Vote::new(1, 1);
    let mut io_state = IOState::new("xx", &vote, applied, snapshot, None);
    if let Some(id) = log_io {
        io_state.log_progress = Valid::new(IOProgress::new_synchronized(Some(id), "xx", "log"));
    }
    io_state
}

#[test]
fn test_pop_if_satisfied() {
    let (r3, _rx3) = new_respond(3);
    let (r5, _rx5) = new_respond(5);
    let mut queue = VecDeque::new();
    queue.push_back(PendingRespond::new(log_id(1, 1, 3), r3));
    queue.push_back(PendingRespond::new(log_id(1, 1, 5), r5));

    assert!(pop_if_satisfied(&mut queue, &log_id(1, 1, 2)).is_none());
    assert_eq!(vec![log_id(1, 1, 3), log_id(1, 1, 5)], waiting(&queue));

    let popped = pop_if_satisfied(&mut queue, &log_id(1, 1, 3));
    assert_eq!(Some(reply(3)), popped.as_ref().map(reply_of));
    assert_eq!(vec![log_id(1, 1, 5)], waiting(&queue));

    let popped = pop_if_satisfied(&mut queue, &log_id(1, 1, 10));
    assert_eq!(Some(reply(5)), popped.as_ref().map(reply_of));
    assert!(queue.is_empty());

    assert!(pop_if_satisfied(&mut queue, &log_id(1, 1, 10)).is_none());
}

#[test]
fn test_pop_if_satisfied_reuses_slot_zero_once_empty() {
    let (r3, _rx3) = new_respond(3);
    let (r4, _rx4) = new_respond(4);
    let mut queue = VecDeque::with_capacity(8);
    queue.push_back(PendingRespond::new(log_id(1, 1, 3), r3));
    let first_slot = queue.as_slices().0.as_ptr();

    assert!(pop_if_satisfied(&mut queue, &log_id(1, 1, 3)).is_some());
    queue.push_back(PendingRespond::new(log_id(1, 1, 4), r4));

    assert_eq!(queue.as_slices().0.as_ptr(), first_slot);
}

#[test]
fn test_drain_satisfied_empty_queues() {
    let mut pending = PendingResponds::<UTConfig>::default();
    let io_state = new_io_state(None, None, None);

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert!(collected.is_empty());
}

#[test]
fn test_drain_satisfied_all_unsatisfied() {
    let (r1, mut rx1) = new_respond(1);
    let (r2, mut rx2) = new_respond(2);
    let mut pending = PendingResponds::<UTConfig>::default();
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 5), r1));
    pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 5), r2));
    let io_state = new_io_state(None, None, None);

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert!(collected.is_empty());

    assert_eq!(vec![log_id(1, 1, 5)], waiting(&pending.on_log_flush));
    assert_eq!(vec![log_id(1, 1, 5)], waiting(&pending.on_apply));
    assert!((&mut rx1).now_or_never().is_none());
    assert!((&mut rx2).now_or_never().is_none());
}

#[test]
fn test_drain_satisfied_log_io() {
    let (r1, _rx1) = new_respond(1);
    let mut pending = PendingResponds::<UTConfig>::default();
    let io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 3)));
    pending.on_log_io.push_back(PendingRespond::new(io_id, r1));

    let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
    let io_state = new_io_state(Some(flushed_io_id), None, None);

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert_eq!(vec![(DrainPhase::LogIO, reply(1))], replies(&collected));
    assert!(pending.on_log_io.is_empty());
}

#[test]
fn test_drain_satisfied_log_flush() {
    let (r1, _rx1) = new_respond(1);
    let mut pending = PendingResponds::<UTConfig>::default();
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), r1));

    let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
    let io_state = new_io_state(Some(flushed_io_id), None, None);

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert_eq!(vec![(DrainPhase::LogFlush, reply(1))], replies(&collected));
    assert!(pending.on_log_flush.is_empty());
}

#[test]
fn test_drain_satisfied_apply() {
    let (r1, _rx1) = new_respond(1);
    let mut pending = PendingResponds::<UTConfig>::default();
    pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 3), r1));

    let io_state = new_io_state(None, Some(log_id(1, 1, 5)), None);

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert_eq!(vec![(DrainPhase::Apply, reply(1))], replies(&collected));
    assert!(pending.on_apply.is_empty());
}

#[test]
fn test_drain_satisfied_snapshot() {
    let (r1, _rx1) = new_respond(1);
    let mut pending = PendingResponds::<UTConfig>::default();
    pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 3), r1));

    let io_state = new_io_state(None, None, Some(log_id(1, 1, 5)));

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert_eq!(vec![(DrainPhase::Snapshot, reply(1))], replies(&collected));
    assert!(pending.on_snapshot.is_empty());
}

/// One satisfied respond in each queue drains in phase order.
#[test]
fn test_drain_satisfied_multiple_phases() {
    let (r1, _rx1) = new_respond(1);
    let (r2, _rx2) = new_respond(2);
    let (r3, _rx3) = new_respond(3);
    let (r4, _rx4) = new_respond(4);
    let mut pending = PendingResponds::<UTConfig>::default();
    let io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 3)));
    pending.on_log_io.push_back(PendingRespond::new(io_id, r1));
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), r2));
    pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 3), r3));
    pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 3), r4));

    let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
    let io_state = new_io_state(Some(flushed_io_id), Some(log_id(1, 1, 5)), Some(log_id(1, 1, 5)));

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert_eq!(
        vec![
            (DrainPhase::LogIO, reply(1)),
            (DrainPhase::LogFlush, reply(2)),
            (DrainPhase::Apply, reply(3)),
            (DrainPhase::Snapshot, reply(4)),
        ],
        replies(&collected)
    );
}

/// The responds for 3 and 5 drain and reach their receivers; the respond for 7 stays queued.
#[test]
fn test_drain_satisfied_partial_satisfaction() {
    let (r3, mut rx3) = new_respond(3);
    let (r5, mut rx5) = new_respond(5);
    let (r7, mut rx7) = new_respond(7);
    let mut pending = PendingResponds::<UTConfig>::default();
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), r3));
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 5), r5));
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 7), r7));

    let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
    let io_state = new_io_state(Some(flushed_io_id), None, None);

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert_eq!(
        vec![(DrainPhase::LogFlush, reply(3)), (DrainPhase::LogFlush, reply(5))],
        replies(&collected)
    );
    assert_eq!(vec![log_id(1, 1, 7)], waiting(&pending.on_log_flush));

    for (_phase, respond) in collected {
        respond.send();
    }
    assert_eq!(Some(reply(3)), (&mut rx3).now_or_never().and_then(Result::ok));
    assert_eq!(Some(reply(5)), (&mut rx5).now_or_never().and_then(Result::ok));
    assert!((&mut rx7).now_or_never().is_none());
}

/// Each queue drains its satisfied front responds in order, and keeps the rest.
#[test]
fn test_drain_satisfied_multiple_items_per_phase() {
    let mut pending = PendingResponds::<UTConfig>::default();
    let mut receivers = Vec::new();
    let mut respond = |tag| {
        let (respond, rx) = new_respond(tag);
        receivers.push(rx);
        respond
    };

    let io_id1 = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 2)));
    let io_id2 = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 3)));
    let io_id3 = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 7)));
    pending.on_log_io.push_back(PendingRespond::new(io_id1, respond(11)));
    pending.on_log_io.push_back(PendingRespond::new(io_id2, respond(12)));
    pending.on_log_io.push_back(PendingRespond::new(io_id3.clone(), respond(13)));

    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 2), respond(21)));
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 3), respond(22)));
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 4), respond(23)));
    pending.on_log_flush.push_back(PendingRespond::new(log_id(1, 1, 8), respond(24)));

    pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 2), respond(31)));
    pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 4), respond(32)));
    pending.on_apply.push_back(PendingRespond::new(log_id(1, 1, 9), respond(33)));

    pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 3), respond(41)));
    pending.on_snapshot.push_back(PendingRespond::new(log_id(1, 1, 10), respond(42)));

    let flushed_io_id = TestIOId::new_log_io(Default::default(), Some(log_id(1, 1, 5)));
    let io_state = new_io_state(Some(flushed_io_id), Some(log_id(1, 1, 5)), Some(log_id(1, 1, 5)));

    let collected: Vec<_> = pending.drain_satisfied(&io_state).collect();
    assert_eq!(
        vec![
            (DrainPhase::LogIO, reply(11)),
            (DrainPhase::LogIO, reply(12)),
            (DrainPhase::LogFlush, reply(21)),
            (DrainPhase::LogFlush, reply(22)),
            (DrainPhase::LogFlush, reply(23)),
            (DrainPhase::Apply, reply(31)),
            (DrainPhase::Apply, reply(32)),
            (DrainPhase::Snapshot, reply(41)),
        ],
        replies(&collected)
    );

    assert_eq!(vec![io_id3], waiting(&pending.on_log_io));
    assert_eq!(vec![log_id(1, 1, 8)], waiting(&pending.on_log_flush));
    assert_eq!(vec![log_id(1, 1, 9)], waiting(&pending.on_apply));
    assert_eq!(vec![log_id(1, 1, 10)], waiting(&pending.on_snapshot));
}

#[test]
fn test_drain_phase_display() {
    assert_eq!(format!("{}", DrainPhase::LogIO), "log_io");
    assert_eq!(format!("{}", DrainPhase::LogFlush), "log_flush");
    assert_eq!(format!("{}", DrainPhase::Apply), "apply");
    assert_eq!(format!("{}", DrainPhase::Snapshot), "snapshot");
    assert_eq!(format!("{}", DrainPhase::Done), "done");
}
