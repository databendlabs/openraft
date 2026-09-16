use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use super::leader_activity_test::engine;
use crate::Membership;
use crate::MembershipState;
use crate::engine::testing::UTConfig;
use crate::proposer::leader_activity::LeaderActivity;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;

#[test]
fn membership_rebuild_preserves_activity_deadline_and_evidence() {
    let mut eng = engine(true);
    let evidence_at = UTConfig::<()>::now();
    let deadline = evidence_at + Duration::from_millis(123);
    eng.leader.as_mut().unwrap().activity = Some(LeaderActivity::Active {
        next_check_at: deadline,
    });
    eng.leader.as_mut().unwrap().update_clock(&1, evidence_at);

    tracing::info!("Rebuild membership without restarting activity or discarding retained evidence");
    {
        let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2, 3, 6}], [4, 5]);
        let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
        eng.state.membership_state = MembershipState::new(stored.clone(), stored);
        eng.replication_handler().rebuild_progresses();

        assert_eq!(
            Some(LeaderActivity::Active {
                next_check_at: deadline,
            }),
            eng.leader.as_ref().unwrap().activity
        );
        assert_eq!(
            Some(evidence_at),
            eng.leader.as_ref().unwrap().clock_progress.try_get(&1).unwrap().val
        );
        assert_eq!(
            None,
            eng.leader.as_ref().unwrap().clock_progress.try_get(&6).unwrap().val
        );
        assert!(eng.output.take_commands().is_empty());
    }
}
