//! Tests of `Engine::elect_if_due()`.
//!
//! The election deadline notifier may report a deadline that has since moved, or one reached after
//! this node changed role or membership. `elect_if_due()` must start an election only when the
//! current state makes one due. Cases that pass `enable_pre_vote = false` start a real election
//! when due: unlike `pre_elect()`, `elect()` does not refuse a valid leader lease, so only the
//! checks in `elect_if_due()` can prevent it.

use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

const LEASE: Duration = Duration::from_millis(500);

/// An election is due strictly after its boundary; this is the smallest step past one.
const ONE_NS: Duration = Duration::from_nanos(1);

/// An engine for node 1 in membership `{1,2}`.
fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(1);
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 1)]);
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 1)),
        Membership::new_with_defaults(vec![btreeset! {1,2}], []),
    )));
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng
}

#[test]
fn test_pre_vote_starts_once_the_deadline_has_passed() {
    let mut eng = eng();
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, Duration::ZERO, Vote::new(1, 2));
    let boundary = now + eng.election_timeout();

    tracing::info!("--- at the boundary itself the election is not yet due");
    {
        eng.elect_if_due(boundary, true, true);

        assert!(eng.pre_candidate_ref().is_none());
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!("--- just after the boundary a Pre-Vote round starts, without changing the vote");
    {
        eng.elect_if_due(boundary + ONE_NS, true, true);

        assert_eq!(Vote::new(1, 2), *eng.state.vote_ref());
        assert_eq!(Vote::new(2, 1), *eng.pre_candidate_ref().unwrap().vote_ref());
        assert!(eng.candidate_ref().is_none());
        assert_eq!(
            vec![Command::SendPreVote {
                vote_req: VoteRequest::new(Vote::new(2, 1), Some(log_id(1, 1, 1)))
            }],
            eng.output.take_commands()
        );
    }
}

#[test]
fn test_election_without_pre_vote_starts_once_the_deadline_has_passed() {
    let mut eng = eng();
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, Duration::ZERO, Vote::new(1, 2));
    let due = now + eng.election_timeout() + ONE_NS;

    eng.elect_if_due(due, true, false);

    assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
    assert!(eng.pre_candidate_ref().is_none());
    assert_eq!(ServerState::Candidate, eng.state.server_state);
    assert_eq!(
        vec![Command::SaveVote { vote: Vote::new(2, 1) }, Command::SendVote {
            vote_req: VoteRequest::new(Vote::new(2, 1), Some(log_id(1, 1, 1)))
        },],
        eng.output.take_commands()
    );
}

/// A heartbeat that refreshes the leader lease moves the deadline later.
#[test]
fn test_stale_deadline_after_a_lease_refresh_is_rejected() {
    let mut eng = eng();
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, LEASE, Vote::new_committed(1, 2));
    let timeout = eng.election_timeout();
    let refreshed_at = now + LEASE;

    tracing::info!("--- a heartbeat refreshes the lease before the old deadline is handled");
    {
        eng.state.vote.touch(refreshed_at, LEASE);

        eng.elect_if_due(now + LEASE + timeout + ONE_NS, true, false);

        assert_eq!(Vote::new_committed(1, 2), *eng.state.vote_ref());
        assert!(eng.candidate_ref().is_none());
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!("--- the refreshed deadline starts the election once it passes");
    {
        eng.elect_if_due(refreshed_at + LEASE + timeout + ONE_NS, true, false);

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(ServerState::Candidate, eng.state.server_state);
    }
}

/// A vote granted to another candidate restarts the wait for an election timeout.
#[test]
fn test_stale_deadline_after_a_vote_change_is_rejected() {
    let mut eng = eng();
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, LEASE, Vote::new_committed(1, 2));
    let timeout = eng.election_timeout();
    let voted_at = now + LEASE + Duration::from_millis(1);

    tracing::info!("--- node 1 grants candidate 2 a vote in term 2 before the old deadline is handled");
    {
        eng.state.vote.update(voted_at, Duration::ZERO, Vote::new(2, 2));

        eng.elect_if_due(now + LEASE + timeout + ONE_NS, true, false);

        assert_eq!(Vote::new(2, 2), *eng.state.vote_ref());
        assert!(eng.candidate_ref().is_none());
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!("--- an election timeout after the new vote, node 1 campaigns in term 3");
    {
        eng.elect_if_due(voted_at + timeout + ONE_NS, true, false);

        assert_eq!(Vote::new(3, 1), *eng.state.vote_ref());
        assert_eq!(ServerState::Candidate, eng.state.server_state);
    }
}

#[test]
fn test_stale_deadline_after_a_membership_change_is_rejected() {
    let mut eng = eng();
    // A vote that was never updated makes the election due at once.
    eng.state.vote = Leased::without_last_update(Vote::new_committed(1, 2));

    tracing::info!("--- node 1 is a learner in both the committed and the effective membership");
    {
        let membership = Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(1, 1, 1)),
            Membership::new_with_defaults(vec![btreeset! {2,3}], [1]),
        ));
        eng.state.membership_state = MembershipState::new(membership.clone(), membership);

        eng.elect_if_due(UTConfig::<()>::now(), true, false);

        assert_eq!(Vote::new_committed(1, 2), *eng.state.vote_ref());
        assert!(eng.candidate_ref().is_none());
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }
}

#[test]
fn test_stale_deadline_after_becoming_leader_is_rejected() {
    let mut eng = eng();
    // Long past any election deadline of the follower node 1 used to be.
    let late = UTConfig::<()>::now() + Duration::from_secs(3600);

    tracing::info!("--- node 1 wins an election in term 2");
    {
        eng.state.vote = Leased::without_last_update(Vote::new_committed(1, 2));
        eng.elect();
        eng.candidate_mut().unwrap().grant_by(&1);
        eng.handle_vote_resp(2, VoteResponse::new(Vote::new(2, 1), Some(log_id(1, 1, 1)), true));
        eng.output.take_commands();

        assert_eq!(ServerState::Leader, eng.state.server_state);
    }

    tracing::info!("--- a deadline notification sent before it became the Leader is ignored");
    {
        eng.elect_if_due(late, true, false);

        assert_eq!(Vote::new_committed(2, 1), *eng.state.vote_ref());
        assert!(eng.leader.is_some(), "leadership is preserved");
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!("--- a leadership transfer in progress keeps the Leader from campaigning");
    {
        eng.leader.as_mut().unwrap().mark_transfer(2, Duration::from_secs(60));

        eng.elect_if_due(late, true, false);

        assert!(eng.leader.is_some(), "leadership is preserved");
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!("--- once the transfer has expired, the Leader steps down and campaigns in term 3");
    {
        eng.leader.as_mut().unwrap().mark_transfer(2, Duration::ZERO);

        eng.elect_if_due(late, true, false);

        assert_eq!(Vote::new(3, 1), *eng.state.vote_ref());
        assert!(eng.leader.is_none(), "the leader steps down");
        assert_eq!(ServerState::Candidate, eng.state.server_state);
    }
}

#[test]
fn test_duplicate_deadline_after_an_election_is_rejected() {
    let mut eng = eng();
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::without_last_update(Vote::new(1, 2));

    tracing::info!("--- the first notification starts an election in term 2");
    {
        eng.elect_if_due(now, true, false);

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        eng.output.take_commands();
    }

    tracing::info!("--- a duplicate notification does not start another election");
    {
        eng.elect_if_due(now, true, false);

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }
}

#[test]
fn test_duplicate_deadline_after_a_pre_vote_is_rejected() {
    let mut eng = eng();
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::without_last_update(Vote::new(1, 2));

    tracing::info!("--- the first notification starts a Pre-Vote round in term 2");
    let started = {
        eng.elect_if_due(now, true, true);

        let pre_candidate = eng.pre_candidate_ref().unwrap();
        assert_eq!(Vote::new(2, 1), *pre_candidate.vote_ref());
        let started = pre_candidate.starting_time();
        eng.output.take_commands();
        started
    };

    tracing::info!("--- a duplicate notification does not restart the round");
    {
        eng.elect_if_due(now, true, true);

        assert_eq!(started, eng.pre_candidate_ref().unwrap().starting_time());
        assert_eq!(Vote::new(1, 2), *eng.state.vote_ref());
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }
}

#[test]
fn test_duplicate_deadline_of_a_single_campaigner_is_rejected() {
    let mut eng = eng();
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 1)),
        Membership::new_with_defaults(vec![btreeset! {1}], []),
    )));
    let now = UTConfig::<()>::now();
    eng.state.vote = Leased::new(now, LEASE, Vote::new(1, 1));

    tracing::info!("--- the single campaigner skips Pre-Vote and the lease wait, and elects in term 2");
    {
        eng.elect_if_due(now, true, true);

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert!(eng.pre_candidate_ref().is_none());
        eng.output.take_commands();
    }

    tracing::info!("--- a duplicate notification does not restart the election in progress");
    {
        eng.elect_if_due(now, true, true);

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }
}

#[test]
fn test_deadline_with_elections_disabled_is_rejected() {
    let mut eng = eng();
    eng.state.vote = Leased::without_last_update(Vote::new_committed(1, 2));
    let now = UTConfig::<()>::now();

    eng.elect_if_due(now, false, false);
    eng.elect_if_due(now, false, true);

    assert_eq!(Vote::new_committed(1, 2), *eng.state.vote_ref());
    assert!(eng.candidate_ref().is_none());
    assert!(eng.pre_candidate_ref().is_none());
    assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
}
