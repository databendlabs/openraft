use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft_state::LogStateReader;

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    // Setup: purged at 2 (leader 2), leader 2's logs at 3-4, leader 4's logs at 5-6
    eng.state.log_ids = LogIdList::new(Some(log_id(2, 1, 2)), vec![log_id(2, 1, 4), log_id(4, 1, 6)]);
    eng
}

#[test]
fn test_purge_log_already_purged() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(1, 1, 1));
    lh.purge_log();

    assert_eq!(Some(&log_id(2, 1, 2)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(2, 1, 2)), lh.state.log_ids.purged());
    assert_eq!(log_id(2, 1, 4), lh.state.log_ids.key_log_ids()[0]);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(0, lh.output.take_commands().len());

    Ok(())
}

#[test]
fn test_purge_log_equal_prev_last_purged() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(2, 1, 2));
    lh.purge_log();

    assert_eq!(Some(&log_id(2, 1, 2)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(2, 1, 2)), lh.state.log_ids.purged());
    assert_eq!(log_id(2, 1, 4), lh.state.log_ids.key_log_ids()[0]);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(0, lh.output.take_commands().len());

    Ok(())
}

#[test]
fn test_purge_log_same_leader_as_prev_last_purged() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    // Purge to 3 (within leader 2's range)
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(2, 1, 3));
    lh.purge_log();

    assert_eq!(Some(&log_id(2, 1, 3)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(2, 1, 3)), lh.state.log_ids.purged());
    // key_log_ids still has leader 2's last at 4, leader 4's last at 6
    assert_eq!(log_id(2, 1, 4), lh.state.log_ids.key_log_ids()[0]);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(2, 1, 3) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_to_last_key_log() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    // Purge to 4 (at leader 2's last)
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(2, 1, 4));
    lh.purge_log();

    assert_eq!(Some(&log_id(2, 1, 4)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(2, 1, 4)), lh.state.log_ids.purged());
    // leader 2 is completely purged, only leader 4 remains
    assert_eq!(log_id(4, 1, 6), lh.state.log_ids.key_log_ids()[0]);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(2, 1, 4) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_go_pass_last_key_log() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    // Purge to 5 (within leader 4's range)
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(4, 1, 5));
    lh.purge_log();

    assert_eq!(Some(&log_id(4, 1, 5)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(4, 1, 5)), lh.state.log_ids.purged());
    // Only leader 4's log at 6 remains
    assert_eq!(log_id(4, 1, 6), lh.state.log_ids.key_log_ids()[0]);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(4, 1, 5) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_to_last_log_id() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    // Purge to 6 (at last log)
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(4, 1, 6));
    lh.purge_log();

    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.log_ids.purged());
    // All logs purged, key_log_ids is empty
    assert!(lh.state.log_ids.key_log_ids().is_empty());
    // last_log_id returns the purged log when all logs are purged
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(4, 1, 6) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_go_pass_last_log_id() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    // Purge to 7 (beyond last log)
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(4, 1, 7));
    lh.purge_log();

    assert_eq!(Some(&log_id(4, 1, 7)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(4, 1, 7)), lh.state.log_ids.purged());
    // All logs purged, key_log_ids is empty
    assert!(lh.state.log_ids.key_log_ids().is_empty());
    // last_log_id returns the purged log when all logs are purged
    assert_eq!(Some(&log_id(4, 1, 7)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(4, 1, 7) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_to_higher_leader_lgo() -> anyhow::Result<()> {
    // Setup: purged at 2, leader 2 at 3-4, leader 4 at 5-6
    // Purge to 7 with higher leader (installing snapshot scenario)
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(5, 1, 7));
    lh.purge_log();

    assert_eq!(Some(&log_id(5, 1, 7)), lh.state.last_purged_log_id());
    assert_eq!(Some(&log_id(5, 1, 7)), lh.state.log_ids.purged());
    // All logs purged, key_log_ids is empty
    assert!(lh.state.log_ids.key_log_ids().is_empty());
    // last_log_id returns the purged log when all logs are purged
    assert_eq!(Some(&log_id(5, 1, 7)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(5, 1, 7) }],
        lh.output.take_commands()
    );

    Ok(())
}
