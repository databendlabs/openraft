use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::Vote;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::InitializeSnapshotError;
use crate::errors::NotInMembers;
use crate::raft_state::LogStateReader;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;

fn membership(node_id: u64) -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {node_id}], [])
}

fn snapshot(last_log_id: Option<LogIdOf<UTConfig>>, node_id: u64) -> SnapshotOf<UTConfig, ()> {
    Snapshot {
        meta: SnapshotMeta {
            last_log_id,
            last_membership: StoredMembershipOf::<UTConfig>::new(last_log_id, membership(node_id)),
        },
        snapshot: (),
    }
}

fn pristine_engine() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(1);
    eng.state.enable_validation(false);
    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_initialize_from_snapshot() -> anyhow::Result<()> {
    let mut eng = pristine_engine();
    let last_log_id = log_id(4, 2, 42);

    let condition = eng.initialize_from_snapshot(Vote::new(5, 1), snapshot(Some(last_log_id), 1))?;

    assert_eq!(Condition::Snapshot { log_id: last_log_id }, condition);
    assert_eq!(&Vote::new(5, 1), eng.state.vote_ref());
    assert!(eng.leader.is_none(), "recovery must not establish a leader");
    assert_eq!(Some(&last_log_id), eng.state.snapshot_last_log_id());
    assert_eq!(&membership(1), eng.state.membership_state.effective().membership());

    eng.elect();
    assert_eq!(
        &Vote::new(6, 1),
        eng.state.vote_ref(),
        "a normal election advances beyond the recovery vote"
    );

    Ok(())
}

#[test]
fn test_initialize_from_snapshot_validates_input() {
    let last_log_id = log_id(4, 2, 42);

    let mut initialized = pristine_engine();
    initialized.initialize(membership(1)).unwrap();
    assert!(matches!(
        initialized.initialize_from_snapshot(Vote::new(5, 1), snapshot(Some(last_log_id), 1)),
        Err(InitializeSnapshotError::NotAllowed(_))
    ));

    assert_eq!(
        Err(InitializeSnapshotError::MissingLastLogId),
        pristine_engine().initialize_from_snapshot(Vote::new(5, 1), snapshot(None, 1))
    );

    assert_eq!(
        Err(InitializeSnapshotError::CommittedVote {
            vote: Vote::new_committed(5, 1),
        }),
        pristine_engine().initialize_from_snapshot(Vote::new_committed(5, 1), snapshot(Some(last_log_id), 1))
    );

    assert_eq!(
        Err(InitializeSnapshotError::VoteForAnotherNode {
            node_id: 1,
            vote: Vote::new(5, 2),
        }),
        pristine_engine().initialize_from_snapshot(Vote::new(5, 2), snapshot(Some(last_log_id), 1))
    );

    assert_eq!(
        Err(InitializeSnapshotError::VoteNotAboveSnapshot {
            vote: Vote::new(4, 1),
            last_log_id,
        }),
        pristine_engine().initialize_from_snapshot(Vote::new(4, 1), snapshot(Some(last_log_id), 1))
    );

    assert_eq!(
        Err(InitializeSnapshotError::NotInMembers(NotInMembers {
            node_id: 1,
            membership: membership(2),
        })),
        pristine_engine().initialize_from_snapshot(Vote::new(5, 1), snapshot(Some(last_log_id), 2))
    );
}
