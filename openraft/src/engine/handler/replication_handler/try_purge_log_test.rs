use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::progress::Inflight;
use crate::progress::inflight_id::InflightId;
use crate::raft_state::LogStateReader;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m23() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2, 3}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(2);
    eng.state.enable_validation(false); // The fixture starts with no pending purge.
    eng.config.id = 2;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 2),
    );
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 2)), m23())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 2, 3)), m23())),
    );
    eng.state.log_ids = LogIdList::new(Some(log_id(1, 1, 2)), [log_id(1, 1, 4), log_id(2, 2, 6)]);
    eng.state.server_state = eng.calc_server_state();
    eng.testing_new_leader();
    eng.output.clear_commands();
    eng
}

#[test]
fn test_try_purge_log_already_purged() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(1, 1, 2));

    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(1, 1, 2)),
            Some(log_id(1, 1, 2)),
            vec![log_id(1, 1, 4), log_id(2, 2, 6)],
            vec![],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}

#[test]
fn test_try_purge_log_postpones_when_logs_are_inflight() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(2, 2, 5));
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::logs(Some(log_id(1, 1, 4)), Some(log_id(2, 2, 6)), InflightId::new(1));
    });

    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(1, 1, 2)),
            Some(log_id(2, 2, 5)),
            vec![log_id(1, 1, 4), log_id(2, 2, 6)],
            vec![],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}

#[test]
fn test_try_purge_log_retries_after_inflight_clears() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(2, 2, 5));
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::logs(Some(log_id(1, 1, 4)), Some(log_id(2, 2, 6)), InflightId::new(1));
    });

    eng.replication_handler().try_purge_log();
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::None;
    });
    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(2, 2, 5)),
            Some(log_id(2, 2, 5)),
            vec![log_id(2, 2, 6)],
            vec![Command::PurgeLog { upto: log_id(2, 2, 5) }],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}

#[test]
fn test_try_purge_log_excludes_inflight_previous_log() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(2, 2, 5));
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::logs(Some(log_id(2, 2, 5)), Some(log_id(2, 2, 6)), InflightId::new(1));
    });

    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(2, 2, 5)),
            Some(log_id(2, 2, 5)),
            vec![log_id(2, 2, 6)],
            vec![Command::PurgeLog { upto: log_id(2, 2, 5) }],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}
