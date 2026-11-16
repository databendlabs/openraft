use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::Vote;
use crate::engine::ElectionDeadline;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

const LEASE: Duration = Duration::from_millis(500);

const NO_WAIT: ElectionDeadline<InstantOf<UTConfig>> = ElectionDeadline {
    lease_expires_at: None,
    pre_vote_retry_at: None,
};

fn m1() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1}], [])
}

fn m12() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

/// An engine for node 1 with `membership` effective.
fn eng(membership: Membership<u64, ()>) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(1);
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 1)]);
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 1)),
        membership,
    )));
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng
}

#[test]
fn test_election_timeout_is_extended_while_a_greater_log_is_known() {
    let mut eng = eng(m12());
    let timer = eng.config.timer_config.clone();

    assert_eq!(timer.election_timeout, eng.election_timeout());

    eng.set_greater_log(Some(log_id(2, 2, 2)));
    assert_eq!(
        timer.election_timeout + timer.smaller_log_timeout,
        eng.election_timeout()
    );

    // Once the local log catches up, the extension no longer applies.
    eng.state.log_ids = LogIdList::new(None, [log_id(2, 2, 2)]);
    assert_eq!(timer.election_timeout, eng.election_timeout());
}

#[test]
fn test_is_single_campaigner() {
    let now = UTConfig::<()>::now();

    let mut sole_voter = eng(m1());
    sole_voter.state.vote = Leased::new(now, LEASE, Vote::new(1, 1));
    assert!(sole_voter.is_single_campaigner(), "sole voter holding its own vote");

    // The sole voter granted its vote to node 2, a campaigner outside the voter set.
    sole_voter.state.vote = Leased::new(now, LEASE, Vote::new(1, 2));
    assert!(
        !sole_voter.is_single_campaigner(),
        "a vote granted to another node is honored"
    );

    let mut one_of_two = eng(m12());
    one_of_two.state.vote = Leased::new(now, LEASE, Vote::new(1, 1));
    assert!(!one_of_two.is_single_campaigner(), "another voter exists");
}

#[test]
fn test_deadline_is_the_vote_lease_extended_by_the_election_timeout() {
    let mut eng = eng(m12());
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, LEASE, Vote::new_committed(1, 2));
    let timer = eng.config.timer_config.clone();

    assert_eq!(
        Some(ElectionDeadline {
            lease_expires_at: Some(now + LEASE + timer.election_timeout),
            pre_vote_retry_at: None,
        }),
        eng.election_deadline(false)
    );

    eng.set_greater_log(Some(log_id(2, 2, 2)));
    assert_eq!(
        Some(ElectionDeadline {
            lease_expires_at: Some(now + LEASE + timer.election_timeout + timer.smaller_log_timeout),
            pre_vote_retry_at: None,
        }),
        eng.election_deadline(false)
    );
}

#[test]
fn test_deadline_without_a_vote_update_is_due_at_once() {
    let mut eng = eng(m12());
    eng.state.vote = Leased::without_last_update(Vote::new(1, 2));

    assert_eq!(Some(NO_WAIT), eng.election_deadline(false));
}

#[test]
fn test_single_campaigner_skips_the_lease_wait() {
    let mut eng = eng(m1());
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, LEASE, Vote::new(1, 1));

    assert_eq!(Some(NO_WAIT), eng.election_deadline(false));

    // An election in progress must not be restarted.
    eng.new_candidate(Vote::new(2, 1));
    assert_eq!(None, eng.election_deadline(false));
}

#[test]
fn test_sole_voter_holding_another_vote_waits_for_the_lease() {
    let mut eng = eng(m1());
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, LEASE, Vote::new(1, 2));
    let election_timeout = eng.config.timer_config.election_timeout;

    assert_eq!(
        Some(ElectionDeadline {
            lease_expires_at: Some(now + LEASE + election_timeout),
            pre_vote_retry_at: None,
        }),
        eng.election_deadline(false)
    );
}

#[test]
fn test_pre_vote_retry_waits_an_election_timeout() {
    let mut eng = eng(m12());
    // A vote that was never updated imposes no lease wait, leaving only the Pre-Vote retry.
    eng.state.vote = Leased::without_last_update(Vote::new(1, 2));
    let timer = eng.config.timer_config.clone();

    assert_eq!(
        Some(NO_WAIT),
        eng.election_deadline(true),
        "no Pre-Vote round is in flight"
    );

    let started = eng.new_pre_candidate(Vote::new(2, 1)).starting_time();
    let deadline = eng.election_deadline(true).unwrap();
    assert_eq!(
        ElectionDeadline {
            lease_expires_at: None,
            pre_vote_retry_at: Some(started + timer.election_timeout),
        },
        deadline
    );
    assert!(!deadline.is_due(started), "a fresh Pre-Vote round is not retried");
    assert!(
        deadline.is_due(started + timer.election_timeout),
        "a round pending for an election timeout is retried"
    );

    eng.set_greater_log(Some(log_id(2, 2, 2)));
    assert_eq!(
        Some(ElectionDeadline {
            lease_expires_at: None,
            pre_vote_retry_at: Some(started + timer.election_timeout + timer.smaller_log_timeout),
        }),
        eng.election_deadline(true),
        "the retry waits for the extended election timeout"
    );

    assert_eq!(
        Some(NO_WAIT),
        eng.election_deadline(false),
        "an election without Pre-Vote ignores the round in flight"
    );
}
