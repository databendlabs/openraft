use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::VoteRequest;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::EffectiveMembershipOf;
use crate::utime::Leased;

fn m1() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1}], [])
}

fn m12() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.log_ids = LogIdList::new(None, [log_id(0, 0, 0)]);
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng
}

#[test]
fn test_elect_single_node() -> anyhow::Result<()> {
    tracing::info!("--- single node: still need to wait for Vote to persist");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.membership_state.set_effective(Arc::new(EffectiveMembershipOf::<UTConfig>::new(
            Some(log_id(0, 1, 1)),
            m1(),
        )));

        eng.elect();

        assert_eq!(Vote::new(1, 1), *eng.state.vote_ref());
        assert!(eng.leader.is_none());
        assert!(eng.candidate_ref().is_some(), "candidate state is pending");

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(1, 1) },
                Command::SendVote {
                    vote_req: VoteRequest {
                        vote: Vote::new(1, 1),
                        last_log_id: Some(log_id(0, 0, 0)),
                        pre_vote: false,
                    },
                },
            ],
            eng.output.take_commands()
        );
    }

    Ok(())
}

#[test]
fn test_elect_single_node_elect_again() -> anyhow::Result<()> {
    tracing::info!("--- single node: electing again will override previous state");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.membership_state.set_effective(Arc::new(EffectiveMembershipOf::<UTConfig>::new(
            Some(log_id(0, 1, 1)),
            m1(),
        )));

        // Build in-progress election state
        eng.state.vote = Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(1, 2),
        );
        eng.testing_new_leader();
        eng.candidate_mut().map(|candidate| candidate.grant_by(&1));

        eng.elect();

        assert_eq!(Vote::new(2, 1), *eng.state.vote_ref());
        assert_eq!(Vote::new(2, 1), *eng.candidate_ref().unwrap().vote_ref());

        assert!(eng.candidate_mut().is_some(), "candidate state is pending");

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(2, 1) },
                Command::SendVote {
                    vote_req: VoteRequest {
                        vote: Vote::new(2, 1),
                        last_log_id: Some(log_id(0, 0, 0)),
                        pre_vote: false,
                    },
                },
            ],
            eng.output.take_commands()
        );
    }
    Ok(())
}

#[test]
fn test_elect_multi_node_enter_candidate() -> anyhow::Result<()> {
    tracing::info!("--- multi nodes: enter candidate state");
    {
        let mut eng = eng();
        eng.config.id = 1;
        eng.state.membership_state.set_effective(Arc::new(EffectiveMembershipOf::<UTConfig>::new(
            Some(log_id(0, 1, 1)),
            m12(),
        )));
        eng.state.log_ids = LogIdList::new(None, vec![log_id(1, 1, 1)]);

        eng.elect();

        assert_eq!(Vote::new(1, 1), *eng.state.vote_ref());
        assert!(eng.leader.is_none());
        assert_eq!(Vote::new(1, 1), *eng.candidate_ref().unwrap().vote_ref());

        assert_eq!(
            Some(btreeset! {},),
            eng.candidate_ref().map(|x| x.granters().collect::<BTreeSet<_>>())
        );

        assert_eq!(ServerState::Candidate, eng.state.server_state);

        assert_eq!(
            vec![
                //
                Command::SaveVote { vote: Vote::new(1, 1) },
                Command::SendVote {
                    vote_req: VoteRequest::new(Vote::new(1, 1), Some(log_id(1, 1, 1)))
                },
            ],
            eng.output.take_commands()
        );
    }
    Ok(())
}

#[test]
fn test_pre_elect_multi_node_does_not_bump_term() -> anyhow::Result<()> {
    // In a 2-node cluster, a single node's pre-vote round only self-grants (no
    // quorum), so it must NOT bump the persisted vote/term and must NOT save a
    // vote — it only emits a pre-vote SendVote probe.
    let mut eng = eng();
    eng.config.id = 1;
    eng.state.membership_state.set_effective(Arc::new(EffectiveMembershipOf::<UTConfig>::new(
        Some(log_id(0, 1, 1)),
        m12(),
    )));

    let vote_before = eng.state.vote_ref().clone();
    eng.pre_elect();

    assert_eq!(
        vote_before,
        *eng.state.vote_ref(),
        "pre-vote must not bump the persisted vote/term"
    );
    assert!(eng.pre_candidate.is_some(), "pre-vote tally exists");
    assert!(
        eng.candidate_ref().is_none(),
        "real candidate must NOT be set during pre-vote"
    );

    let cmds = eng.output.take_commands();
    assert!(
        cmds.iter().any(|c| matches!(c, Command::SendVote { vote_req } if vote_req.pre_vote)),
        "must emit a pre-vote SendVote probe"
    );
    assert!(
        !cmds.iter().any(|c| matches!(c, Command::SaveVote { .. })),
        "pre-vote must not persist a vote"
    );
    Ok(())
}

#[test]
fn test_pre_elect_single_node_proceeds_to_real_election() -> anyhow::Result<()> {
    // A single-voter cluster reaches quorum on its own pre-vote, so it proceeds
    // straight to the real election: the term is bumped and the vote is saved.
    let mut eng = eng();
    eng.config.id = 1;
    eng.state.membership_state.set_effective(Arc::new(EffectiveMembershipOf::<UTConfig>::new(
        Some(log_id(0, 1, 1)),
        m1(),
    )));

    eng.pre_elect();

    assert_eq!(Vote::new(1, 1), *eng.state.vote_ref(), "real election bumped the term");
    assert!(
        eng.pre_candidate.is_none(),
        "pre-vote tally cleared after proceeding to real election"
    );

    let cmds = eng.output.take_commands();
    assert!(
        cmds.iter().any(|c| matches!(c, Command::SaveVote { .. })),
        "real election must save the vote"
    );
    assert!(
        cmds.iter().any(|c| matches!(c, Command::SendVote { vote_req } if !vote_req.pre_vote)),
        "real election sends a normal (non-pre) vote"
    );
    Ok(())
}
