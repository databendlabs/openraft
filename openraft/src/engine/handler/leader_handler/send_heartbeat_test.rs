use std::sync::Arc;

use maplit::btreeset;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;
#[allow(unused_imports)] use pretty_assertions::assert_str_eq;

use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::TokioInstant;
use crate::Vote;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], btreeset! {1,2,3})
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.committed = Some(log_id(0, 1, 0));
    eng.state.vote = UTime::new(TokioInstant::now(), Vote::new_committed(3, 1));
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_leader_send_heartbeat() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.vote_handler().become_leading();

    // A heartbeat is a normal AppendEntries RPC if there are pending data to send.
    {
        eng.leader_handler()?.send_heartbeat();
        assert_eq!(
            vec![
                Command::Replicate {
                    target: 2,
                    req: Inflight::logs(None, Some(log_id(2, 1, 3))).with_id(1),
                },
                Command::Replicate {
                    target: 3,
                    req: Inflight::logs(None, Some(log_id(2, 1, 3))).with_id(1),
                },
            ],
            eng.output.take_commands()
        );
    }

    // No RPC will be sent if there are inflight RPC
    {
        eng.output.clear_commands();
        eng.leader_handler()?.send_heartbeat();
        assert!(eng.output.take_commands().is_empty());
    }

    // No data to send, sending a heartbeat is to send empty RPC:
    {
        let l = eng.leader_handler()?;
        let _ = l.leader.progress.update_with(&2, |ent| ent.update_matching(1, Some(log_id(2, 1, 3))).unwrap());
        let _ = l.leader.progress.update_with(&3, |ent| ent.update_matching(1, Some(log_id(2, 1, 3))).unwrap());
    }
    eng.output.clear_commands();
    eng.leader_handler()?.send_heartbeat();
    assert_eq!(
        vec![
            Command::Replicate {
                target: 2,
                req: Inflight::logs(Some(log_id(2, 1, 3)), Some(log_id(2, 1, 3))).with_id(1),
            },
            Command::Replicate {
                target: 3,
                req: Inflight::logs(Some(log_id(2, 1, 3)), Some(log_id(2, 1, 3))).with_id(1),
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
