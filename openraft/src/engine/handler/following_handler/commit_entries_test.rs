use std::sync::Arc;

use maplit::btreeset;

use crate::engine::handler::following_handler::FollowingHandler;
use crate::engine::testing::UTCfg;
use crate::engine::CEngine;
use crate::engine::Command;
use crate::engine::Engine;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::testing::DummyConfig;
use crate::BasicNode;
use crate::EffectiveMembership;
use crate::Entry;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;

fn m01() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {2,3}], None)
}

fn eng() -> CEngine<UTCfg> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.state.committed = Some(log_id(1, 1));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
    );
    eng
}

#[test]
fn test_following_handler_max_committed() -> anyhow::Result<()> {
    let f = FollowingHandler::<u64, BasicNode, Entry<DummyConfig>>::max_committed;

    assert_eq!(None, f(None, None, None));

    assert_eq!(None, f(None, Some(log_id(1, 2)), None));
    assert_eq!(None, f(None, None, Some(log_id(1, 2))));
    assert_eq!(None, f(None, Some(log_id(1, 2)), Some(log_id(1, 3))));

    assert_eq!(Some(log_id(1, 2)), f(Some(log_id(1, 3)), Some(log_id(1, 2)), None));
    assert_eq!(Some(log_id(1, 3)), f(Some(log_id(1, 4)), None, Some(log_id(1, 3))));
    assert_eq!(
        Some(log_id(1, 3)),
        f(Some(log_id(1, 3)), Some(log_id(1, 2)), Some(log_id(1, 4)))
    );
    assert_eq!(
        Some(log_id(1, 4)),
        f(Some(log_id(1, 5)), Some(log_id(1, 2)), Some(log_id(1, 4)))
    );
    assert_eq!(Some(log_id(1, 2)), f(Some(log_id(1, 5)), Some(log_id(1, 2)), None));
    Ok(())
}

#[test]
fn test_following_handler_commit_entries_empty() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(None);

    assert_eq!(Some(&log_id(1, 1)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_following_handler_commit_entries_no_update() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(Some(log_id(1, 1)));

    assert_eq!(Some(&log_id(1, 1)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_following_handler_commit_entries_gt_last_entry() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.following_handler().commit_entries(Some(log_id(2, 3)));

    assert_eq!(Some(&log_id(2, 3)), eng.state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        ),
        eng.state.membership_state
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::FollowerCommit {
                already_committed: Some(log_id(1, 1)),
                upto: log_id(2, 3)
            }, //
        ],
        eng.output.take_commands()
    );

    Ok(())
}
