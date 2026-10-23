use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::ServerState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteRequest;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m_empty() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {}], [])
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn m34() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {3,4}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false);
    eng.config.id = 2;
    // This will be overridden
    eng.state.server_state = ServerState::default();
    eng
}

/// It is a Leader but not yet append any logs.
#[test]
fn test_startup_as_leader_without_logs() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 3)),
        m23(),
    )));
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 3)]);
    // The persisted vote identifies node 2 as the former leader.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 2),
    );

    eng.startup();

    assert_eq!(ServerState::Candidate, eng.state.server_state);
    let has_leader = eng.leader_ref().is_some();
    assert!(!has_leader);
    let vote = Vote::new(3, 2);
    let actual_vote = eng.state.vote_ref();
    assert_eq!(&vote, actual_vote);

    let last_log_id = Some(log_id(1, 1, 3));
    let vote_req = VoteRequest::new(vote, last_log_id);
    let want = vec![Command::SaveVote { vote }, Command::SendVote { vote_req }];
    let commands = eng.output.take_commands();
    assert_eq!(want, commands);

    Ok(())
}

#[test]
fn test_startup_as_leader_with_proposed_logs() -> anyhow::Result<()> {
    tracing::info!("--- a leader proposed logs and restarted, campaign in a new term");
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(2, 1, 3)),
        m23(),
    )));
    // Last-per-leader format: leader (1,1) last at index 3, leader (1,2) last at index 6
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 3), log_id(1, 2, 6)]);
    // The persisted vote identifies node 2 as the former leader.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(1, 2),
    );

    eng.startup();

    assert_eq!(ServerState::Candidate, eng.state.server_state);
    let has_leader = eng.leader_ref().is_some();
    assert!(!has_leader);
    let vote = Vote::new(2, 2);
    let actual_vote = eng.state.vote_ref();
    assert_eq!(&vote, actual_vote);

    let last_log_id = Some(log_id(1, 2, 6));
    let vote_req = VoteRequest::new(vote, last_log_id);
    let want = vec![Command::SaveVote { vote }, Command::SendVote { vote_req }];
    let commands = eng.output.take_commands();
    assert_eq!(want, commands);

    Ok(())
}

/// When starting up, a leader that is not a voter should not panic.
#[test]
fn test_startup_as_leader_not_voter_issue_920() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(2, 1, 3)),
        m_empty(),
    )));
    // Committed vote makes it a leader at startup.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(1, 2),
    );

    eng.startup();

    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(eng.output.take_commands(), vec![]);

    Ok(())
}

#[test]
fn test_startup_candidate_becomes_follower() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(2, 1, 3)),
        m23(),
    )));
    // Non-committed vote makes it a candidate at startup.
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2));

    eng.startup();

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
#[test]
fn test_startup_as_follower() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(2, 1, 3)),
        m23(),
    )));

    eng.startup();

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_startup_as_learner() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is not a voter:
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(2, 1, 3)),
        m34(),
    )));

    eng.startup();

    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_startup_as_leader_leader_restore_disabled() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.config.enable_leader_restore = false;

    // self.id==2 is a voter:
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        Some(log_id(1, 1, 3)),
        m23(),
    )));
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 3)]);
    // Committed vote would normally make it a leader at startup.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 2),
    );

    eng.startup();

    // It does not re-elect itself; as a voter it becomes a Follower.
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert!(eng.leader_ref().is_none());
    assert!(!eng.state.is_leader(&eng.config.id));
    // The demotion is in-memory only: no command is emitted; the committed vote stays in storage.
    assert_eq!(eng.output.take_commands(), vec![]);
    assert_eq!(eng.state.vote_ref(), &Vote::new(2, 2));

    Ok(())
}

#[test]
fn test_startup_leader_removed_from_effective_membership() {
    for (restore, committed_index, retain_as_learner, expected_state, expected_vote) in [
        (true, 3, false, ServerState::Candidate, Vote::new(3, 2)),
        (false, 3, false, ServerState::Learner, Vote::new(2, 2)),
        (true, 5, false, ServerState::Learner, Vote::new_committed(2, 2)),
        (true, 5, true, ServerState::Learner, Vote::new(2, 2)),
    ] {
        tracing::info!(
            restore,
            committed_index,
            retain_as_learner,
            ?expected_state,
            "--- restart node 2 with joint {{2,3}},{{3}} at index 3 and final {{3}} at index 5"
        );
        {
            let mut eng = eng();
            eng.config.enable_leader_restore = restore;
            eng.state.log_ids = LogIdList::new(None, [log_id(1, 3, 5)]);
            eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::ZERO, Vote::new_committed(2, 2));
            eng.state.membership_state = MembershipState::new(
                Arc::new(StoredMembershipOf::<UTConfig>::new(
                    Some(log_id(1, 3, 3)),
                    Membership::new_with_defaults(vec![btreeset! {2,3}, btreeset! {3}], []),
                )),
                Arc::new(StoredMembershipOf::<UTConfig>::new(
                    Some(log_id(1, 3, 5)),
                    Membership::new_with_defaults(vec![btreeset! {3}], retain_as_learner.then_some(2)),
                )),
            );
            eng.state.update_local_committed(&Some(log_id(1, 3, committed_index)));

            eng.startup();

            assert_eq!(expected_state, eng.state.server_state);
            let actual_vote = eng.state.vote_ref();
            assert_eq!(&expected_vote, actual_vote);
            let expect_candidate = expected_state == ServerState::Candidate;
            let has_candidate = eng.candidate_ref().is_some();
            assert_eq!(expect_candidate, has_candidate);
            let has_leader = eng.leader_ref().is_some();
            assert!(!has_leader);
            let commands = eng.output.take_commands();
            if expect_candidate {
                let last_log_id = Some(log_id(1, 3, 5));
                let vote_req = VoteRequest::new(expected_vote, last_log_id);
                let want = vec![Command::SaveVote { vote: expected_vote }, Command::SendVote {
                    vote_req,
                }];
                assert_eq!(want, commands);
            } else {
                let want: Vec<Command<UTConfig>> = vec![];
                assert_eq!(want, commands);
            }
        }
    }
}
