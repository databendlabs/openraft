use std::ops::Deref;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::StoredMembership;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::progress::Progress;
use crate::storage::SnapshotMeta;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;

fn m12() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng.state.membership_state = MembershipState::new(
        EffectiveMembership::new_arc(Some(log_id(1, 0, 1)), m12()),
        EffectiveMembership::new_arc(Some(log_id(1, 0, 1)), m12()),
    );

    eng.state.log_ids = LogIdList::new([log_id(0, 0, 0)]);
    eng
}

#[test]
fn test_trigger_purge_log_no_snapshot() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.trigger_purge_log(1);

    assert_eq!(None, eng.state.purge_upto, "no snapshot, cannot purge");

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_trigger_purge_log_already_scheduled() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.snapshot_meta = SnapshotMeta {
        last_log_id: Some(log_id(1, 0, 3)),
        last_membership: StoredMembership::new(Some(log_id(1, 0, 1)), m12()),
        snapshot_id: "1".to_string(),
    };
    eng.state.purge_upto = Some(log_id(1, 0, 2));
    eng.state.io_state.purged = Some(log_id(1, 0, 2));

    eng.trigger_purge_log(2);

    assert_eq!(Some(log_id(1, 0, 2)), eng.state.purge_upto, "already purged, no update");

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_trigger_purge_log_delete_only_in_snapshot_logs() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.snapshot_meta = SnapshotMeta {
        last_log_id: Some(log_id(1, 0, 3)),
        last_membership: StoredMembership::new(Some(log_id(1, 0, 1)), m12()),
        snapshot_id: "1".to_string(),
    };
    eng.state.purge_upto = Some(log_id(1, 0, 2));
    eng.state.io_state.purged = Some(log_id(1, 0, 2));
    eng.state.log_ids = LogIdList::new([log_id(1, 0, 2), log_id(1, 0, 10)]);

    eng.trigger_purge_log(5);

    assert_eq!(
        Some(log_id(1, 0, 3)),
        eng.state.purge_upto,
        "delete only in snapshot logs"
    );

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(1, 0, 3) },],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_trigger_purge_log_in_used_wont_be_delete() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.snapshot_meta = SnapshotMeta {
        last_log_id: Some(log_id(1, 0, 3)),
        last_membership: StoredMembership::new(Some(log_id(1, 0, 1)), m12()),
        snapshot_id: "1".to_string(),
    };
    eng.state.purge_upto = Some(log_id(1, 0, 2));
    eng.state.io_state.purged = Some(log_id(1, 0, 2));
    eng.state.log_ids = LogIdList::new([log_id(1, 0, 2), log_id(1, 0, 10)]);
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );

    // Make it a leader and mark the logs are in flight.
    eng.testing_new_leader();
    let l = eng.leader.as_mut().unwrap();
    let _ = l.progress.get_mut(&2).unwrap().next_send(eng.state.deref(), 10).unwrap();

    eng.trigger_purge_log(5);

    assert_eq!(
        Some(log_id(1, 0, 3)),
        eng.state.purge_upto,
        "delete only in snapshot logs"
    );

    assert_eq!(0, eng.output.take_commands().len(), "in used log wont be deleted");

    Ok(())
}
