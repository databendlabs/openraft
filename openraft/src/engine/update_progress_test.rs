use std::sync::Arc;

use maplit::btreeset;

use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::Vote;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m01() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {0,1}], None)
}

fn m123() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1,2,3}], None)
}

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64> {
        id: 2, // make it a member
        ..Default::default()
    };
    eng.state.vote = Vote::new_committed(2, 1);
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m123()));
    eng
}

#[test]
fn test_update_progress_no_leader() -> anyhow::Result<()> {
    let mut eng = eng();

    // eng.state.leader is None.

    let eng0 = eng.clone();
    eng.update_progress(3, Some(log_id(1, 2)));

    assert_eq!(eng0, eng, "nothing changed");

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_update_progress_update_leader_progress() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.new_leader();

    // progress: None, None, (1,2)
    eng.update_progress(3, Some(log_id(1, 2)));
    assert_eq!(None, eng.state.committed);

    // progress: None, (2,1), (1,2); quorum-ed: (1,2), not at leader vote, not committed
    eng.update_progress(2, Some(log_id(2, 1)));
    assert_eq!(None, eng.state.committed);
    assert_eq!(0, eng.commands.len());

    // progress: None, (2,1), (2,3); committed: (2,1)
    eng.update_progress(3, Some(log_id(2, 3)));
    assert_eq!(Some(log_id(2, 1)), eng.state.committed);
    assert_eq!(
        vec![
            Command::ReplicateCommitted {
                committed: Some(log_id(2, 1))
            },
            Command::LeaderCommit {
                since: None,
                upto: log_id(2, 1)
            }
        ],
        eng.commands
    );

    eng.commands = vec![];
    // progress: (2,4), (2,1), (2,3); committed: (1,3)
    eng.update_progress(1, Some(log_id(2, 4)));
    assert_eq!(Some(log_id(2, 3)), eng.state.committed);
    assert_eq!(
        vec![
            Command::ReplicateCommitted {
                committed: Some(log_id(2, 3))
            },
            Command::LeaderCommit {
                since: Some(log_id(2, 1)),
                upto: log_id(2, 3)
            }
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_update_progress_purge_upto_committed() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new([log_id(2, 0), log_id(2, 5)]);
    eng.config.max_applied_log_to_keep = 0;
    eng.config.purge_batch_size = 1;

    eng.state.new_leader();

    // progress: None, (2,1), (2,3); committed: (2,1)
    eng.update_progress(3, Some(log_id(1, 2)));
    eng.update_progress(2, Some(log_id(2, 1)));
    eng.update_progress(3, Some(log_id(2, 3)));
    assert_eq!(Some(log_id(2, 1)), eng.state.committed);
    assert_eq!(
        vec![
            Command::ReplicateCommitted {
                committed: Some(log_id(2, 1))
            },
            Command::LeaderCommit {
                since: None,
                upto: log_id(2, 1)
            },
            Command::PurgeLog { upto: log_id(2, 1) },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_update_progress_purge_upto_committed_minus_1() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new([log_id(2, 0), log_id(2, 5)]);
    eng.config.max_applied_log_to_keep = 1;
    eng.config.purge_batch_size = 1;

    eng.state.new_leader();

    // progress: None, (2,1), (2,3); committed: (2,1)
    eng.update_progress(3, Some(log_id(1, 2)));
    eng.update_progress(2, Some(log_id(2, 2)));
    eng.update_progress(3, Some(log_id(2, 4)));
    assert_eq!(Some(log_id(2, 2)), eng.state.committed);
    assert_eq!(
        vec![
            Command::ReplicateCommitted {
                committed: Some(log_id(2, 2))
            },
            Command::LeaderCommit {
                since: None,
                upto: log_id(2, 2)
            },
            Command::PurgeLog { upto: log_id(2, 1) },
        ],
        eng.commands
    );

    Ok(())
}
