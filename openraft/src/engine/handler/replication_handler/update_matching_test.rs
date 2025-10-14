use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m123() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m123())),
    );

    eng
}

#[test]
fn test_update_matching_no_leader() -> anyhow::Result<()> {
    // There is no leader, it should panic.

    let res = std::panic::catch_unwind(move || {
        let mut eng = eng();
        eng.replication_handler().update_matching(3, Some(log_id(1, 1, 2)));
    });
    tracing::info!("res: {:?}", res);
    assert!(res.is_err());

    Ok(())
}

#[test]
fn test_update_matching() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.testing_new_leader();
    eng.output.take_commands();

    let mut rh = eng.replication_handler();
    {
        let prog_entry = rh.leader.progress.get_mut(&1).unwrap();
        prog_entry.inflight = Inflight::logs(Some(log_id(2, 1, 3)), Some(log_id(2, 1, 4)));
    };
    {
        let prog_entry = rh.leader.progress.get_mut(&2).unwrap();
        prog_entry.inflight = Inflight::logs(Some(log_id(1, 1, 0)), Some(log_id(2, 1, 4)));
    };
    {
        let prog_entry = rh.leader.progress.get_mut(&3).unwrap();
        prog_entry.inflight = Inflight::logs(Some(log_id(1, 1, 1)), Some(log_id(2, 1, 4)));
    };

    // progress: None, None, (1,2)
    {
        rh.update_matching(3, Some(log_id(1, 1, 2)));
        assert_eq!(None, rh.state.committed());
        assert_eq!(0, rh.output.take_commands().len());
    }

    // progress: None, (2,1), (1,2); quorum-ed: (1,2), not at leader vote, not committed
    {
        rh.output.clear_commands();
        rh.update_matching(2, Some(log_id(2, 1, 1)));
        assert_eq!(None, rh.state.committed());
        assert_eq!(0, rh.output.take_commands().len());
    }

    // progress: None, (2,1), (2,3); committed: (2,1)
    {
        rh.output.clear_commands();
        rh.update_matching(3, Some(log_id(2, 1, 3)));
        assert_eq!(Some(&log_id(2, 1, 1)), rh.state.committed());
        assert_eq!(
            vec![Command::ReplicateCommitted {
                committed: Some(log_id(2, 1, 1))
            },],
            rh.output.take_commands()
        );
        assert_eq!(Some(&log_id(2, 1, 1)), rh.state.io_state.apply_progress.accepted());
        assert_eq!(None, rh.state.io_state.apply_progress.submitted());
    }

    // progress: (2,4), (2,1), (2,3); committed: (1,3)
    {
        rh.output.clear_commands();
        rh.update_matching(1, Some(log_id(2, 1, 4)));
        assert_eq!(Some(&log_id(2, 1, 3)), rh.state.committed());
        assert_eq!(
            vec![Command::ReplicateCommitted {
                committed: Some(log_id(2, 1, 3))
            },],
            rh.output.take_commands()
        );
        assert_eq!(Some(&log_id(2, 1, 3)), rh.state.io_state.apply_progress.accepted());
        assert_eq!(None, rh.state.io_state.apply_progress.submitted());
    }

    Ok(())
}
