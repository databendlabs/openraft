use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::ReplicationProgress;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::progress::Inflight;
use crate::progress::Progress;
use crate::progress::entry::ProgressEntry;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;

fn m01() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn m23_45() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {2,3}], btreeset! {4,5})
}

fn m34() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {3,4}], [])
}

fn m4_356() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {4}], btreeset! {3,5,6})
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.config.id = 2;
    eng.state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(6, 2),
    );
    eng.state.server_state = eng.calc_server_state();
    eng
}

#[test]
fn test_leader_append_membership_for_leader() -> anyhow::Result<()> {
    let mut eng = eng();
    // Make it a real leader: voted for itself and vote is committed.
    eng.testing_new_leader();
    eng.output.take_commands();

    eng.replication_handler().append_membership(&log_id(3, 1, 4), &m34());

    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m34()))
        ),
        eng.state.membership_state
    );
    assert_eq!(
        ServerState::Leader,
        eng.state.server_state,
        "Leader wont be affected by membership change"
    );

    assert_eq!(
        vec![
            //
            Command::RebuildReplicationStreams {
                targets: vec![
                    ReplicationProgress(3, ProgressEntry::empty(0)),
                    ReplicationProgress(4, ProgressEntry::empty(0))
                ], /* node-2 is leader,
                    * won't be removed */
            }
        ],
        eng.output.take_commands()
    );

    assert!(
        eng.leader.as_ref().unwrap().progress.get(&4).matching().is_none(),
        "exists, but it is a None"
    );

    Ok(())
}

#[test]
fn test_leader_append_membership_update_learner_process() -> anyhow::Result<()> {
    // When updating membership, voter progress should inherit from learner progress, and
    // learner process should inherit from voter process. If voter changes to
    // learner or vice versa.

    let mut eng = eng();
    eng.state.log_ids = LogIdList::new([log_id(0, 0, 0), log_id(1, 1, 1), log_id(5, 1, 10)]);

    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23_45())));

    // Make it a real leader: voted for itself and vote is committed.
    eng.testing_new_leader();

    if let Some(l) = &mut eng.leader.as_mut() {
        assert_eq!(&ProgressEntry::empty(11), l.progress.get(&4));
        assert_eq!(&ProgressEntry::empty(11), l.progress.get(&5));

        let p = ProgressEntry::new(Some(log_id(1, 1, 4)));
        let _ = l.progress.update(&4, p.clone());
        assert_eq!(&p, l.progress.get(&4));

        let p = ProgressEntry::new(Some(log_id(1, 1, 5)));
        let _ = l.progress.update(&5, p.clone());
        assert_eq!(&p, l.progress.get(&5));

        let p = ProgressEntry::new(Some(log_id(1, 1, 3)));
        let _ = l.progress.update(&3, p.clone());
        assert_eq!(&p, l.progress.get(&3));
    } else {
        unreachable!("leader should not be None");
    }

    eng.replication_handler().append_membership(&log_id(3, 1, 4), &m4_356());

    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23_45())),
            Arc::new(EffectiveMembership::new(Some(log_id(3, 1, 4)), m4_356()))
        ),
        eng.state.membership_state
    );

    if let Some(l) = &mut eng.leader.as_mut() {
        assert_eq!(
            &ProgressEntry::new(Some(log_id(1, 1, 4)))
                .with_inflight(Inflight::logs(Some(log_id(1, 1, 4)), Some(log_id(5, 1, 10)))),
            l.progress.get(&4),
            "learner-4 progress should be transferred to voter progress"
        );

        assert_eq!(
            &ProgressEntry::new(Some(log_id(1, 1, 3)))
                .with_inflight(Inflight::logs(Some(log_id(1, 1, 3)), Some(log_id(5, 1, 10)))),
            l.progress.get(&3),
            "voter-3 progress should be transferred to learner progress"
        );

        assert_eq!(
            &ProgressEntry::new(Some(log_id(1, 1, 5)))
                .with_inflight(Inflight::logs(Some(log_id(1, 1, 5)), Some(log_id(5, 1, 10)))),
            l.progress.get(&5),
            "learner-5 has previous value"
        );

        assert_eq!(
            &ProgressEntry::empty(11).with_inflight(Inflight::logs(None, Some(log_id(5, 1, 10)))),
            l.progress.get(&6)
        );
    } else {
        unreachable!("leader should not be None");
    }

    Ok(())
}
