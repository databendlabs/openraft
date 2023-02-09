use std::sync::Arc;

use maplit::btreeset;

use crate::engine::Command;
use crate::engine::Engine;
use crate::progress::entry::ProgressEntry;
use crate::progress::Inflight;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::ServerState;
use crate::Vote;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::default();
    eng.config.id = 2;
    // This will be overridden
    eng.state.server_state = ServerState::default();
    eng
}

#[test]
fn test_startup_as_leader() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())));
    // Committed vote makes it a leader at startup.
    eng.state.vote = Vote::new_committed(1, 2);

    eng.startup();

    assert_eq!(ServerState::Leader, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: true,
            local_data: false,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            //
            Command::BecomeLeader,
            Command::RebuildReplicationStreams {
                targets: vec![(3, ProgressEntry {
                    matching: None,
                    curr_inflight_id: 0,
                    inflight: Inflight::None,
                    searching_end: 0
                })]
            }
        ],
        eng.output.commands
    );

    Ok(())
}

#[test]
fn test_startup_candidate_becomes_follower() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())));
    // Non-committed vote makes it a candidate at startup.
    eng.state.vote = Vote::new(1, 2);

    eng.startup();

    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.commands.len());

    Ok(())
}
#[test]
fn test_startup_as_follower() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())));

    eng.startup();

    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.commands.len());

    Ok(())
}

#[test]
fn test_startup_as_learner() -> anyhow::Result<()> {
    let mut eng = eng();
    // self.id==2 is not a voter:
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m34())));

    eng.startup();

    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(0, eng.output.commands.len());

    Ok(())
}
