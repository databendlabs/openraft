use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::ReplicationProgress;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::log_id_range::LogIdRange;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::raft_state::IOId;
use crate::replication::request::Replicate;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::EntryOf;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    eng.state.server_state = ServerState::Candidate;
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())));

    eng.output.take_commands();
    eng
}

#[test]
fn test_become_leader() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.vote_handler().become_leader();

    let leader = eng.leader.as_ref().unwrap();
    assert_eq!(leader.noop_log_id, log_id(2, 1, 0));
    assert_eq!(leader.last_log_id(), Some(&log_id(2, 1, 0)));
    assert_eq!(*leader.committed_vote_ref(), Vote::new(2, 1).into_committed());

    assert_eq!(ServerState::Leader, eng.state.server_state);

    assert_eq!(eng.output.take_commands(), vec![
        Command::UpdateIOProgress {
            when: None,
            io_id: IOId::new_log_io(Vote::new(2, 1).into_committed(), None)
        },
        Command::RebuildReplicationStreams {
            targets: vec![ReplicationProgress(0, ProgressEntry::empty(0))]
        },
        Command::AppendEntries {
            committed_vote: Vote::new(2, 1).into_committed(),
            entries: vec![EntryOf::<UTConfig>::new_blank(log_id(2, 1, 0)),]
        },
        Command::Replicate {
            target: 0,
            req: Replicate::logs(LogIdRange::new(None, Some(log_id(2, 1, 0))), InflightId::new(1))
        }
    ]);

    Ok(())
}
