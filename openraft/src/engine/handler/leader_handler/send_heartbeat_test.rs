use std::sync::Arc;

use maplit::btreeset;
#[allow(unused_imports)] use pretty_assertions::assert_eq;
#[allow(unused_imports)] use pretty_assertions::assert_ne;
#[allow(unused_imports)] use pretty_assertions::assert_str_eq;
use tokio::time::Instant;

use crate::engine::Command;
use crate::engine::Engine;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), NodeId=u64, Node=(), Entry = crate::Entry<Foo>
);

use crate::testing::log_id;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.committed = Some(log_id(0, 0));
    eng.state.vote = UTime::new(Instant::now(), Vote::new_committed(3, 1));
    eng.state.log_ids.append(log_id(1, 1));
    eng.state.log_ids.append(log_id(2, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23())),
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
                    req: Inflight::logs(None, Some(log_id(2, 3))).with_id(1),
                },
                Command::Replicate {
                    target: 3,
                    req: Inflight::logs(None, Some(log_id(2, 3))).with_id(1),
                },
            ],
            eng.output.commands
        );
    }

    // No RPC will be sent if there are inflight RPC
    {
        eng.output.commands = vec![];
        eng.leader_handler()?.send_heartbeat();
        assert!(eng.output.commands.is_empty());
    }

    // No data to send, sending a heartbeat is to send empty RPC:
    {
        let l = eng.leader_handler()?;
        let _ = l.leader.progress.update_with(&2, |ent| ent.update_matching(Some(log_id(2, 3))));
        let _ = l.leader.progress.update_with(&3, |ent| ent.update_matching(Some(log_id(2, 3))));
    }
    eng.output.commands = vec![];
    eng.leader_handler()?.send_heartbeat();
    assert_eq!(
        vec![
            Command::Replicate {
                target: 2,
                req: Inflight::logs(Some(log_id(2, 3)), Some(log_id(2, 3))).with_id(1),
            },
            Command::Replicate {
                target: 3,
                req: Inflight::logs(Some(log_id(2, 3)), Some(log_id(2, 3))).with_id(1),
            },
        ],
        eng.output.commands
    );

    Ok(())
}
