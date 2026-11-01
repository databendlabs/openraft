use std::panic::AssertUnwindSafe;
use std::panic::catch_unwind;
use std::sync::Arc;
use std::time::Duration;

use super::LeaseReadStatePublisher;
use crate::Vote;
use crate::async_runtime::watch::RecvError;
use crate::async_runtime::watch::WatchReceiver;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::linearizable_read::lease_read_state::LeaseReadState;
use crate::type_config::TypeConfigExt;
use crate::vote::raft_vote::RaftVoteExt;

type C = UTConfig;

fn snapshot() -> LeaseReadState<C> {
    let now = C::now();
    let vote = Vote::new_committed(2, 1);
    LeaseReadState {
        leader_vote: vote.to_committed(),
        noop_log_id: log_id(2, 1, 3),
        committed: Some(log_id(2, 1, 5)),
        quorum_acked_at: Some(now),
        leader_lease: Duration::from_millis(100),
        self_quorum: false,
        applied: Some(log_id(2, 1, 4)),
    }
}

#[test]
fn publication_preserves_the_complete_snapshot() {
    let (tx, rx) = C::watch_channel(None);
    let publisher = LeaseReadStatePublisher::<C>::new(tx);
    let state = snapshot();
    let changed = publisher.publish(Some(state.clone()));
    assert!(changed);
    let actual = rx.borrow_watched().clone();
    let expected = Some(Arc::new(state));
    assert_eq!(expected, actual);
}

#[test]
fn identical_publication_preserves_the_arc() {
    let (tx, rx) = C::watch_channel(None);
    let publisher = LeaseReadStatePublisher::<C>::new(tx);
    let state = snapshot();
    publisher.publish(Some(state.clone()));
    let first = rx.borrow_watched().clone();
    let first = first.unwrap();
    let changed = publisher.publish(Some(state));
    assert!(!changed);
    let second = rx.borrow_watched().clone();
    let second = second.unwrap();
    let same_allocation = Arc::ptr_eq(&first, &second);
    assert!(same_allocation);
}

#[test]
fn identical_publication_does_not_notify() {
    C::run(async {
        let (tx, mut rx) = C::watch_channel(None);
        let publisher = LeaseReadStatePublisher::<C>::new(tx);
        let state = snapshot();
        publisher.publish(Some(state.clone()));
        let current = rx.borrow_and_update();
        drop(current);
        publisher.publish(Some(state));
        let changed = rx.changed();
        futures_util::pin_mut!(changed);
        let polled = futures_util::poll!(changed);
        let pending = polled.is_pending();
        assert!(pending);
    });
}

#[test]
fn quorum_acknowledgement_may_regress() {
    let (tx, rx) = C::watch_channel(None);
    let publisher = LeaseReadStatePublisher::<C>::new(tx);
    let mut state = snapshot();
    publisher.publish(Some(state.clone()));
    state.quorum_acked_at = None;
    let changed = publisher.publish(Some(state.clone()));
    assert!(changed);
    let actual = rx.borrow_watched().clone();
    let expected = Some(Arc::new(state));
    assert_eq!(expected, actual);
}

#[test]
fn clearing_is_idempotent() {
    let (tx, rx) = C::watch_channel(None);
    let publisher = LeaseReadStatePublisher::<C>::new(tx);
    let state = snapshot();
    publisher.publish(Some(state));
    let changed = publisher.clear();
    assert!(changed);
    let actual = rx.borrow_watched().clone();
    assert_eq!(None, actual);
    let changed = publisher.clear();
    assert!(!changed);
}

#[test]
fn dropping_the_publisher_invalidates_retained_receivers() {
    let (tx, rx) = C::watch_channel(None);
    let publisher = LeaseReadStatePublisher::<C>::new(tx);
    let state = snapshot();
    publisher.publish(Some(state));
    drop(publisher);
    let actual = rx.borrow_watched().clone();
    assert_eq!(None, actual);
}

#[test]
fn unwinding_invalidates_the_snapshot() {
    let (tx, rx) = C::watch_channel(None);
    let publisher = LeaseReadStatePublisher::<C>::new(tx);
    let state = snapshot();
    publisher.publish(Some(state));
    let unwind = AssertUnwindSafe(move || {
        let _publisher = publisher;
        panic!("stop the core while it owns the publisher");
    });
    let result = catch_unwind(unwind);
    let panicked = result.is_err();
    assert!(panicked);
    let actual = rx.borrow_watched().clone();
    assert_eq!(None, actual);
}

#[test]
fn dropping_the_publisher_closes_the_channel_after_invalidation() {
    C::run(async {
        let (tx, mut rx) = C::watch_channel(None);
        let publisher = LeaseReadStatePublisher::<C>::new(tx);
        let state = snapshot();
        publisher.publish(Some(state));
        drop(publisher);
        let actual = rx.borrow_and_update().clone();
        assert_eq!(None, actual);
        let received = rx.changed().await;
        let expected = Err(RecvError(()));
        assert_eq!(expected, received);
    });
}
