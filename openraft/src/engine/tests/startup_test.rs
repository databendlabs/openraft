use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Entry;
use crate::Membership;
use crate::ServerState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::ReplicationProgress;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::entry::RaftEntry;
use crate::log_id_range::LogIdRange;
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::raft_state::IOId;
use crate::replication::request::Replicate;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m_empty() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn m34() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {3,4}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false);
    eng.config.id = 2;
    // This will be overridden
    eng.state.server_state = ServerState::default();
    eng
}

/// It is a Leader but not yet append any logs.
#[test]
fn test_startup_as_leader_without_logs() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 3)), m23())));
    eng.state.log_ids = LogIdList::new([log_id(1, 1, 3)]);
    // Committed vote makes it a leader at startup.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 2),
    );

    eng.startup();

    assert_eq!(ServerState::Leader, eng.state.server_state);
    let leader = eng.leader_ref().unwrap();
    assert_eq!(leader.noop_log_id(), &log_id(2, 2, 4));
    assert_eq!(leader.last_log_id(), Some(&log_id(2, 2, 4)));
    assert_eq!(
        vec![
            Command::UpdateIOProgress {
                when: None,
                io_id: IOId::new_log_io(Vote::new(2, 2).into_committed(), Some(log_id(1, 1, 3)))
            },
            Command::RebuildReplicationStreams {
                targets: vec![ReplicationProgress(3, ProgressEntry {
                    matching: None,
                    inflight: Inflight::None,
                    searching_end: 4,
                    allow_log_reversion: false,
                })]
            },
            Command::AppendEntries {
                committed_vote: Vote::new(2, 2).into_committed(),
                entries: vec![Entry::<UTConfig>::new_blank(log_id(2, 2, 4))],
            },
            Command::Replicate {
                target: 3,
                req: Replicate::logs(LogIdRange::new(None, Some(log_id(2, 2, 4))), InflightId::new(1)),
            }
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_startup_as_leader_with_proposed_logs() -> anyhow::Result<()> {
    tracing::info!("--- a leader proposed logs and restarted, reuse noop_log_id");
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())));
    // Fake existing log ids
    eng.state.log_ids = LogIdList::new([log_id(1, 1, 2), log_id(1, 2, 4), log_id(1, 2, 6)]);
    // Committed vote makes it a leader at startup.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(1, 2),
    );

    eng.startup();

    assert_eq!(ServerState::Leader, eng.state.server_state);
    let leader = eng.leader_ref().unwrap();
    assert_eq!(leader.noop_log_id(), &log_id(1, 2, 4));
    assert_eq!(leader.last_log_id(), Some(&log_id(1, 2, 6)));
    assert_eq!(
        vec![
            Command::UpdateIOProgress {
                when: None,
                io_id: IOId::new_log_io(Vote::new(1, 2).into_committed(), Some(log_id(1, 2, 6)))
            },
            Command::RebuildReplicationStreams {
                targets: vec![ReplicationProgress(3, ProgressEntry {
                    matching: None,
                    inflight: Inflight::None,
                    searching_end: 7,
                    allow_log_reversion: false,
                })]
            },
            Command::Replicate {
                target: 3,
                req: Replicate::logs(LogIdRange::new(None, Some(log_id(1, 2, 6))), InflightId::new(1))
            }
        ],
        eng.output.take_commands()
    );

    Ok(())
}

/// When starting up, a leader that is not a voter should not panic.
#[test]
fn test_startup_as_leader_not_voter_issue_920() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m_empty())));
    // Committed vote makes it a leader at startup.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(1, 2),
    );

    eng.startup();

    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(eng.output.take_commands(), vec![]);

    Ok(())
}

#[test]
fn test_startup_candidate_becomes_follower() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())));
    // Non-committed vote makes it a candidate at startup.
    eng.state.vote = Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2));

    eng.startup();

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
#[test]
fn test_startup_as_follower() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())));

    eng.startup();

    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_startup_as_learner() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is not a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m34())));

    eng.startup();

    assert_eq!(ServerState::Learner, eng.state.server_state);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
