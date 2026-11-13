use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::linearizable_read::ReadLogId;
use crate::raft_state::LogStateReader;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1,2,3})
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.testing_new_leader();
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_get_linearizer() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
    let noop = log_id(3, 1, 4);
    eng.leader.as_mut().unwrap().noop_log_id = noop;

    let linearizer = eng.try_leader_handler()?.get_linearizer();
    assert_eq!(&1, linearizer.node_id());
    assert_eq!(&noop, linearizer.noop_log_id());
    assert_eq!(Some(&log_id(0, 1, 0)), linearizer.committed_log_id());
    assert_eq!(eng.state.io_applied(), linearizer.applied());
    assert_eq!(ReadLogId::new(noop, None), linearizer.read_log_id());

    let committed = log_id(3, 1, 5);
    eng.state.apply_progress_mut().accept(committed);
    let linearizer = eng.try_leader_handler()?.get_linearizer();
    assert_eq!(&noop, linearizer.noop_log_id());
    assert_eq!(Some(&committed), linearizer.committed_log_id());
    assert_eq!(ReadLogId::new(noop, Some(committed)), linearizer.read_log_id());

    Ok(())
}
