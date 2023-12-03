use std::ops::Deref;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::progress::Progress;
use crate::testing::log_id;
use crate::CommittedLeaderId;
use crate::EffectiveMembership;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::SnapshotMeta;
use crate::StoredMembership;

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng.state.membership_state = MembershipState::new(
        EffectiveMembership::new_arc(Some(log_id(1, 0, 1)), m12()),
        EffectiveMembership::new_arc(Some(log_id(1, 0, 1)), m12()),
    );

    eng.state.log_ids = LogIdList::new([LogId::new(CommittedLeaderId::new(0, 0), 0)]);
    eng
}

#[test]
fn test_trigger_purge_log_no_snapshot() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.trigger_purge_log(1);

    assert_eq!(None, eng.state.purge_upto, "no snapshot, can not purge");

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

    // Make it a leader and mark the logs are in flight.
    eng.vote_handler().become_leading();
    let l = eng.internal_server_state.leading_mut().unwrap();
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
