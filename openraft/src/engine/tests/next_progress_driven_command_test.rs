use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft_state::IOId;
use crate::raft_state::LogStateReader;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::SnapshotMetaOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::vote::raft_vote::RaftVoteExt;

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}], [])
}

/// A follower of leader `(1,1)` that holds and has submitted `0..=40`, committed and applied
/// through 10.
fn eng() -> Engine<UTConfig> {
    let mut eng: Engine<UTConfig> = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    let vote = Vote::new_committed(1, 1);
    eng.state.vote.update(UTConfig::<()>::now(), Duration::from_millis(500), vote);
    eng.state.log_ids = LogIdList::new(None, vec![log_id(1, 1, 40)]);

    let log_io = IOId::new_log_io(vote.to_committed(), Some(log_id(1, 1, 40)));
    eng.state.log_progress_mut().accept(log_io.clone());
    eng.state.log_progress_mut().submit(log_io);

    eng.state.apply_progress_mut().accept(log_id(1, 1, 10));
    eng.state.apply_progress_mut().submit(log_id(1, 1, 10));

    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_no_apply_while_snapshot_install_is_queued() -> anyhow::Result<()> {
    let mut eng = eng();

    // Accepting a snapshot at 30 purges the log ids up to 30 and commits 30 at once, but apply
    // progress moves only when RaftCore runs the queued `InstallFullSnapshot`.
    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(1, 1, 30)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 0)), m12()),
        },
        snapshot: (),
    });
    assert!(cond.is_some());
    assert_eq!(Some(&log_id(1, 1, 30)), eng.state.last_purged_log_id());
    assert_eq!(Some(&log_id(1, 1, 10)), eng.state.apply_progress().submitted());

    assert_eq!(
        None,
        eng.next_progress_driven_command(),
        "11..=30 are purged and the queued install covers them"
    );

    // RaftCore runs the install, then the leader commits 40.
    eng.state.apply_progress_mut().submit(log_id(1, 1, 30));
    eng.state.update_local_committed(&Some(log_id(1, 1, 40)));

    assert_eq!(
        Some(Command::SaveCommittedAndApply {
            already_applied: Some(log_id(1, 1, 30)),
            upto: log_id(1, 1, 40),
        }),
        eng.next_progress_driven_command()
    );

    Ok(())
}
