use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::LeaderId;
use crate::LogId;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::<u64, ()>::default();
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 2), log_id(4, 4), log_id(4, 6)]);
    eng
}

#[test]
fn test_purge_log_already_purged() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(1, 1));

    assert_eq!(Some(log_id(2, 2)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(2, 2), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_purge_log_equal_prev_last_purged() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(2, 2));

    assert_eq!(Some(log_id(2, 2)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(2, 2), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);

    assert_eq!(0, eng.commands.len());

    Ok(())
}
#[test]
fn test_purge_log_same_leader_as_prev_last_purged() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(2, 3));

    assert_eq!(Some(log_id(2, 3)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(2, 3), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);

    assert_eq!(vec![Command::PurgeLog { upto: log_id(2, 3) }], eng.commands);

    Ok(())
}

#[test]
fn test_purge_log_to_last_key_log() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(4, 4));

    assert_eq!(Some(log_id(4, 4)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(4, 4), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);

    assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 4) }], eng.commands);

    Ok(())
}

#[test]
fn test_purge_log_go_pass_last_key_log() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(4, 5));

    assert_eq!(Some(log_id(4, 5)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(4, 5), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);

    assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 5) }], eng.commands);

    Ok(())
}

#[test]
fn test_purge_log_to_last_log_id() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(4, 6));

    assert_eq!(Some(log_id(4, 6)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(4, 6), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);

    assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 6) }], eng.commands);

    Ok(())
}

#[test]
fn test_purge_log_go_pass_last_log_id() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(4, 7));

    assert_eq!(Some(log_id(4, 7)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(4, 7), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(4, 7)), eng.state.last_log_id(),);

    assert_eq!(vec![Command::PurgeLog { upto: log_id(4, 7) }], eng.commands);

    Ok(())
}

#[test]
fn test_purge_log_to_higher_leader_lgo() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.purge_log(log_id(5, 7));

    assert_eq!(Some(log_id(5, 7)), eng.state.last_purged_log_id(),);
    assert_eq!(log_id(5, 7), eng.state.log_ids.key_log_ids()[0],);
    assert_eq!(Some(log_id(5, 7)), eng.state.last_log_id(),);

    assert_eq!(vec![Command::PurgeLog { upto: log_id(5, 7) }], eng.commands);

    Ok(())
}
