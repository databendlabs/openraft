use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::StoredMembership;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::storage::SnapshotMeta;

fn m12() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn m1234() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.state.snapshot_meta = SnapshotMeta {
        last_log_id: Some(log_id(2, 1, 2)),
        last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m12()),
        snapshot_id: "1-2-3-4".to_string(),
    };
    eng
}

#[test]
fn test_update_snapshot_no_update() -> anyhow::Result<()> {
    // snapshot will not be updated because of equal or less `last_log_id`.
    let mut eng = eng();

    let got = eng.snapshot_handler().update_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(2, 1, 2)),
        last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(false, got);

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_update_snapshot_updated() -> anyhow::Result<()> {
    // snapshot will be updated to a new one with greater `last_log_id`.
    let mut eng = eng();

    let got = eng.snapshot_handler().update_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(2, 1, 3)),
        last_membership: StoredMembership::new(Some(log_id(2, 1, 2)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(true, got);

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 1, 3)),
            last_membership: StoredMembership::new(Some(log_id(2, 1, 2)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );

    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
