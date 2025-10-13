use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::testing::blank_ent;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::VoteOf;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng: Engine<UTConfig> = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    let vote = VoteOf::<UTConfig>::new_committed(2, 1);
    let now = UTConfig::<()>::now();
    eng.state.vote.update(now, Duration::from_millis(500), vote);
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_follower_append_entries_update_accepted() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.output.clear_commands();

    eng.following_handler().append_entries(Some(log_id(2, 1, 3)), vec![
        //
        blank_ent(3, 1, 4),
        blank_ent(3, 1, 5),
    ]);

    assert_eq!(
        &[
            log_id(1, 1, 1), //
            log_id(2, 1, 3),
            log_id(3, 1, 4),
            log_id(3, 1, 5),
        ],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(
        Some(&IOId::new_log_io(
            Vote::new(2, 1).into_committed(),
            Some(log_id(3, 1, 5))
        )),
        eng.state.accepted_log_io()
    );
    assert_eq!(eng.output.take_commands(), vec![
        //
        Command::AppendEntries {
            committed_vote: Vote::new(2, 1).into_committed(),
            entries: vec![blank_ent(3, 1, 4), blank_ent(3, 1, 5),],
        }
    ]);

    // Update to a new Leader and smaller log id
    {
        // Assume this node's Leader becomes T3-N1
        eng.state.vote = Leased::new(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(3, 1),
        );
        eng.following_handler().append_entries(Some(log_id(2, 1, 3)), vec![
            //
            blank_ent(3, 1, 4),
        ]);
        assert_eq!(Some(&log_id(3, 1, 5)), eng.state.last_log_id());
        assert_eq!(
            Some(&IOId::new_log_io(
                Vote::new(3, 1).into_committed(),
                Some(log_id(3, 1, 4))
            )),
            eng.state.accepted_log_io()
        );
        assert_eq!(eng.output.take_commands(), vec![
            //
            Command::UpdateIOProgress {
                when: Some(Condition::IOFlushed {
                    io_id: IOId::new_log_io(Vote::new(2, 1).into_committed(), Some(log_id(3, 1, 5)))
                }),
                io_id: IOId::new_log_io(Vote::new(3, 1).into_committed(), Some(log_id(3, 1, 4))),
            }
        ]);
    }

    // Update to a smaller value is ignored.
    {
        eng.following_handler().append_entries(Some(log_id(2, 1, 3)), vec![]);
        assert_eq!(Some(&log_id(3, 1, 5)), eng.state.last_log_id());
        assert_eq!(
            Some(&IOId::new_log_io(
                Vote::new(3, 1).into_committed(),
                Some(log_id(3, 1, 4))
            )),
            eng.state.accepted_log_io()
        );

        assert_eq!(eng.output.take_commands(), vec![
            //
        ]);
    }

    Ok(())
}
