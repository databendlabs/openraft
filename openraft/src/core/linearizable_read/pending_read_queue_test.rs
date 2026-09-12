use std::time::Duration;

use super::*;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::ForwardToLeader;
use crate::errors::LinearizableReadError;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::linearizable_read::ReadLogId;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::OneshotReceiverOf;

type C = UTConfig;
type ReadResult = Result<Linearizer<C>, LinearizableReadError<C>>;
type ReadRx = OneshotReceiverOf<C, ReadResult>;

const CLOCK_ADVANCE: Duration = Duration::from_nanos(1);

struct PendingReadSpec {
    min_quorum_acked_at: InstantOf<C>,
    deadline: InstantOf<C>,
    node_id: u64,
}

fn push_read(queue: &mut PendingReadQueue<C>, read_log_id: &ReadLogId<C>, spec: PendingReadSpec) -> ReadRx {
    let (tx, rx) = C::oneshot();
    let linearizer = Linearizer::new(spec.node_id, *read_log_id, None);
    let pending_read = PendingRead::new(spec.deadline, linearizer, tx);
    queue.push(spec.min_quorum_acked_at, pending_read);
    rx
}

fn assert_linearizer(rx: &mut ReadRx, node_id: u64, read_log_id: &ReadLogId<C>) {
    let linearizer = rx.try_recv().unwrap().unwrap();
    assert_eq!(&node_id, linearizer.node_id());
    assert_eq!(read_log_id, linearizer.read_log_id());
    assert_eq!(None, linearizer.applied());
}

#[test]
fn test_drain_all_with_error() {
    let mut queue = PendingReadQueue::<C>::default();
    let read_log_id = ReadLogId::from_log_id(log_id(1, 1, 1));
    let linearizer = Linearizer::new(1, read_log_id, None);
    let now = C::now();
    let mut receivers = Vec::new();

    for _ in 0..2 {
        let (tx, rx) = C::oneshot::<ReadResult>();
        let pending_read = PendingRead::new(now, linearizer.clone(), tx);
        queue.push(now, pending_read);
        receivers.push(rx);
    }

    let want = ForwardToLeader::new(2, ());
    let err = LinearizableReadError::ForwardToLeader(want.clone());
    queue.drain_all_with_error(err);

    assert!(queue.is_empty());
    for receiver in &mut receivers {
        let response = receiver.try_recv().unwrap();
        let err = response.unwrap_err();
        let LinearizableReadError::ForwardToLeader(got) = err else {
            panic!("expected ForwardToLeader");
        };
        assert_eq!(want, got);
    }
}

#[test]
fn test_drain_satisfied_requires_newer_ack_and_keeps_duplicates() {
    let mut queue = PendingReadQueue::<C>::default();
    let read_log_id = ReadLogId::from_log_id(log_id(1, 1, 1));
    let now = C::now();
    let deadline = now + Duration::from_secs(1);
    let lower_threshold = now + Duration::from_millis(10);
    let higher_threshold = now + Duration::from_millis(20);

    let mut rx1 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: higher_threshold,
        deadline,
        node_id: 1,
    });
    let mut rx2 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: lower_threshold,
        deadline,
        node_id: 2,
    });
    let mut rx3 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: lower_threshold,
        deadline,
        node_id: 3,
    });

    queue.drain_satisfied(lower_threshold, None);

    assert!(rx1.try_recv().is_err());
    assert!(rx2.try_recv().is_err());
    assert!(rx3.try_recv().is_err());

    let lower_acked_at = lower_threshold + CLOCK_ADVANCE;
    queue.drain_satisfied(lower_acked_at, None);

    assert!(rx1.try_recv().is_err());
    assert_linearizer(&mut rx2, 2, &read_log_id);
    assert_linearizer(&mut rx3, 3, &read_log_id);

    let higher_acked_at = higher_threshold + CLOCK_ADVANCE;
    queue.drain_satisfied(higher_acked_at, None);

    assert_linearizer(&mut rx1, 1, &read_log_id);
    assert!(queue.is_empty());
}

#[test]
fn test_drain_expired_removes_expired_prefix() {
    let mut queue = PendingReadQueue::<C>::default();
    let read_log_id = ReadLogId::from_log_id(log_id(1, 1, 1));
    let now = C::now();
    let timeout = Duration::from_millis(100);
    let threshold1 = now;
    let threshold2 = now + Duration::from_millis(10);
    let threshold3 = now + Duration::from_millis(20);
    let deadline1 = threshold1 + timeout;
    let deadline2 = threshold2 + timeout;
    let deadline3 = threshold3 + timeout;

    let mut rx1 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold1,
        deadline: deadline1,
        node_id: 1,
    });
    let mut rx2 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold2,
        deadline: deadline2,
        node_id: 2,
    });
    let mut rx3 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold3,
        deadline: deadline3,
        node_id: 3,
    });

    assert_eq!(Some(deadline1), queue.earliest_deadline());

    let want = ForwardToLeader::new(4, ());
    let mut thresholds = Vec::new();
    queue.drain_expired(deadline2, |min_quorum_acked_at| {
        thresholds.push(min_quorum_acked_at);
        LinearizableReadError::ForwardToLeader(want.clone())
    });

    assert_eq!(vec![threshold1, threshold2], thresholds);

    for receiver in [&mut rx1, &mut rx2] {
        let response = receiver.try_recv().unwrap();
        let LinearizableReadError::ForwardToLeader(got) = response.unwrap_err() else {
            panic!("expected ForwardToLeader");
        };
        assert_eq!(want, got);
    }
    assert_eq!(Some(deadline3), queue.earliest_deadline());

    let quorum_acked_at = threshold3 + CLOCK_ADVANCE;
    queue.drain_satisfied(quorum_acked_at, None);
    assert_linearizer(&mut rx3, 3, &read_log_id);
    assert!(queue.is_empty());
}

/// A per-request wait timeout lets the read with the earlier threshold hold the later deadline,
/// so expiry must follow deadline order instead of threshold order.
#[test]
fn test_drain_expired_follows_deadline_order_not_threshold_order() {
    let mut queue = PendingReadQueue::<C>::default();
    let read_log_id = ReadLogId::from_log_id(log_id(1, 1, 1));
    let now = C::now();

    let threshold1 = now + Duration::from_millis(10);
    let deadline1 = now + Duration::from_millis(300);
    let threshold2 = now + Duration::from_millis(20);
    let deadline2 = now + Duration::from_millis(100);

    let mut rx1 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold1,
        deadline: deadline1,
        node_id: 1,
    });
    let mut rx2 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold2,
        deadline: deadline2,
        node_id: 2,
    });

    assert_eq!(Some(deadline2), queue.earliest_deadline());

    let want = ForwardToLeader::new(3, ());
    let mut thresholds = Vec::new();
    queue.drain_expired(deadline2, |min_quorum_acked_at| {
        thresholds.push(min_quorum_acked_at);
        LinearizableReadError::ForwardToLeader(want.clone())
    });

    assert_eq!(vec![threshold2], thresholds);

    let response = rx2.try_recv().unwrap();
    let LinearizableReadError::ForwardToLeader(got) = response.unwrap_err() else {
        panic!("expected ForwardToLeader");
    };
    assert_eq!(want, got);

    assert!(rx1.try_recv().is_err());
    assert_eq!(Some(deadline1), queue.earliest_deadline());

    let quorum_acked_at = threshold1 + CLOCK_ADVANCE;
    queue.drain_satisfied(quorum_acked_at, None);
    assert_linearizer(&mut rx1, 1, &read_log_id);
    assert!(queue.is_empty());
}

/// Answering a read from the middle of the deadline order must leave both indexes describing the
/// same remaining reads.
#[test]
fn test_satisfied_removes_read_from_the_middle_of_deadline_order() {
    let mut queue = PendingReadQueue::<C>::default();
    let read_log_id = ReadLogId::from_log_id(log_id(1, 1, 1));
    let now = C::now();

    let threshold1 = now + Duration::from_millis(10);
    let deadline1 = now + Duration::from_millis(200);
    let threshold2 = now + Duration::from_millis(20);
    let deadline2 = now + Duration::from_millis(100);
    let threshold3 = now + Duration::from_millis(30);
    let deadline3 = now + Duration::from_millis(300);

    let mut rx1 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold1,
        deadline: deadline1,
        node_id: 1,
    });
    let mut rx2 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold2,
        deadline: deadline2,
        node_id: 2,
    });
    let mut rx3 = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: threshold3,
        deadline: deadline3,
        node_id: 3,
    });

    let quorum_acked_at = threshold1 + CLOCK_ADVANCE;
    queue.drain_satisfied(quorum_acked_at, None);
    assert_linearizer(&mut rx1, 1, &read_log_id);
    assert_eq!(Some(deadline2), queue.earliest_deadline());

    let want = ForwardToLeader::new(4, ());
    let mut thresholds = Vec::new();
    queue.drain_expired(deadline3, |min_quorum_acked_at| {
        thresholds.push(min_quorum_acked_at);
        LinearizableReadError::ForwardToLeader(want.clone())
    });

    assert_eq!(vec![threshold2, threshold3], thresholds);

    for receiver in [&mut rx2, &mut rx3] {
        let response = receiver.try_recv().unwrap();
        let LinearizableReadError::ForwardToLeader(got) = response.unwrap_err() else {
            panic!("expected ForwardToLeader");
        };
        assert_eq!(want, got);
    }

    assert_eq!(None, queue.earliest_deadline());
    assert!(queue.is_empty());
}

/// A read queued before the state machine advanced must report the applied log id observed when it
/// is answered, not the one observed when it was queued.
#[test]
fn test_drain_satisfied_refreshes_applied() {
    let mut queue = PendingReadQueue::<C>::default();
    let read_log_id = ReadLogId::from_log_id(log_id(1, 1, 1));
    let now = C::now();

    let mut rx = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: now,
        deadline: now + Duration::from_millis(100),
        node_id: 1,
    });

    let applied = log_id(1, 1, 5);
    let quorum_acked_at = now + CLOCK_ADVANCE;
    queue.drain_satisfied(quorum_acked_at, Some(applied));

    let linearizer = rx.try_recv().unwrap().unwrap();
    assert_eq!(&1, linearizer.node_id());
    assert_eq!(&read_log_id, linearizer.read_log_id());
    assert_eq!(Some(&log_id(1, 1, 5)), linearizer.applied());
}

#[test]
fn test_satisfied_takes_precedence_over_expired() {
    let mut queue = PendingReadQueue::<C>::default();
    let read_log_id = ReadLogId::from_log_id(log_id(1, 1, 1));
    let now = C::now();
    let mut rx = push_read(&mut queue, &read_log_id, PendingReadSpec {
        min_quorum_acked_at: now,
        deadline: now,
        node_id: 1,
    });

    let quorum_acked_at = now + CLOCK_ADVANCE;
    queue.drain_satisfied(quorum_acked_at, None);

    let err = LinearizableReadError::ForwardToLeader(ForwardToLeader::new(2, ()));
    queue.drain_expired(now, |_| err.clone());

    assert_linearizer(&mut rx, 1, &read_log_id);
    assert!(queue.is_empty());
}
