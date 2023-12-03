use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.state.log_ids = LogIdList::new(vec![log_id(2, 1, 2), log_id(4, 1, 4), log_id(4, 1, 6)]);
    eng.state.purged_next = 3;
    eng
}

#[test]
fn test_purge_log_already_purged() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(1, 1, 1));
    lh.purge_log();

    assert_eq!(Some(&log_id(2, 1, 2)), lh.state.last_purged_log_id());
    assert_eq!(log_id(2, 1, 2), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(0, lh.output.take_commands().len());

    Ok(())
}

#[test]
fn test_purge_log_equal_prev_last_purged() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(2, 1, 2));
    lh.purge_log();

    assert_eq!(Some(&log_id(2, 1, 2)), lh.state.last_purged_log_id());
    assert_eq!(log_id(2, 1, 2), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(0, lh.output.take_commands().len());

    Ok(())
}
#[test]
fn test_purge_log_same_leader_as_prev_last_purged() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(2, 1, 3));
    lh.purge_log();

    assert_eq!(Some(&log_id(2, 1, 3)), lh.state.last_purged_log_id());
    assert_eq!(log_id(2, 1, 3), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(2, 1, 3) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_to_last_key_log() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(4, 1, 4));
    lh.purge_log();

    assert_eq!(Some(&log_id(4, 1, 4)), lh.state.last_purged_log_id());
    assert_eq!(log_id(4, 1, 4), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(4, 1, 4) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_go_pass_last_key_log() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(4, 1, 5));
    lh.purge_log();

    assert_eq!(Some(&log_id(4, 1, 5)), lh.state.last_purged_log_id());
    assert_eq!(log_id(4, 1, 5), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(4, 1, 5) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_to_last_log_id() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(4, 1, 6));
    lh.purge_log();

    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_purged_log_id());
    assert_eq!(log_id(4, 1, 6), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(4, 1, 6)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(4, 1, 6) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_go_pass_last_log_id() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(4, 1, 7));
    lh.purge_log();

    assert_eq!(Some(&log_id(4, 1, 7)), lh.state.last_purged_log_id());
    assert_eq!(log_id(4, 1, 7), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(4, 1, 7)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(4, 1, 7) }],
        lh.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_purge_log_to_higher_leader_lgo() -> anyhow::Result<()> {
    let mut eng = eng();

    let mut lh = eng.log_handler();
    lh.state.purge_upto = Some(log_id(5, 1, 7));
    lh.purge_log();

    assert_eq!(Some(&log_id(5, 1, 7)), lh.state.last_purged_log_id());
    assert_eq!(log_id(5, 1, 7), lh.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(&log_id(5, 1, 7)), lh.state.last_log_id());

    assert_eq!(
        vec![Command::PurgeLog { upto: log_id(5, 1, 7) }],
        lh.output.take_commands()
    );

    Ok(())
}
