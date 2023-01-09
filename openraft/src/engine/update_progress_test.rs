use std::sync::Arc;

use maplit::btreeset;

use crate::engine::Command;
use crate::engine::Engine;
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

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn m123() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2,3}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.config.id = 2;
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

    assert_eq!(0, eng.output.commands.len());

    Ok(())
}

#[test]
fn test_update_progress_update_leader_progress() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.new_leader();

    // progress: None, None, (1,2)
    eng.update_progress(3, Some(log_id(1, 2)));
    assert_eq!(None, eng.state.committed);
    assert_eq!(
        vec![
            //
            Command::UpdateReplicationMetrics {
                target: 3,
                matching: log_id(1, 2),
            },
        ],
        eng.output.commands
    );

    // progress: None, (2,1), (1,2); quorum-ed: (1,2), not at leader vote, not committed
    eng.output.commands = vec![];
    eng.update_progress(2, Some(log_id(2, 1)));
    assert_eq!(None, eng.state.committed);
    assert_eq!(0, eng.output.commands.len());

    // progress: None, (2,1), (2,3); committed: (2,1)
    eng.output.commands = vec![];
    eng.update_progress(3, Some(log_id(2, 3)));
    assert_eq!(Some(log_id(2, 1)), eng.state.committed);
    assert_eq!(
        vec![
            Command::UpdateReplicationMetrics {
                target: 3,
                matching: log_id(2, 3),
            },
            Command::ReplicateCommitted {
                committed: Some(log_id(2, 1))
            },
            Command::LeaderCommit {
                already_committed: None,
                upto: log_id(2, 1)
            }
        ],
        eng.output.commands
    );

    eng.output.commands = vec![];
    // progress: (2,4), (2,1), (2,3); committed: (1,3)
    eng.update_progress(1, Some(log_id(2, 4)));
    assert_eq!(Some(log_id(2, 3)), eng.state.committed);
    assert_eq!(
        vec![
            Command::UpdateReplicationMetrics {
                target: 1,
                matching: log_id(2, 4),
            },
            Command::ReplicateCommitted {
                committed: Some(log_id(2, 3))
            },
            Command::LeaderCommit {
                already_committed: Some(log_id(2, 1)),
                upto: log_id(2, 3)
            }
        ],
        eng.output.commands
    );

    Ok(())
}
