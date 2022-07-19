use std::sync::Arc;

use maplit::btreeset;

use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m1() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1}], None)
}

fn m123_345() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {1,2,3}, btreeset! {3,4,5}], None)
}

#[test]
fn test_membership_state_is_member() -> anyhow::Result<()> {
    let x = MembershipState {
        committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1())),
        effective: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m123_345())),
    };

    assert!(!x.is_voter(&0));
    assert!(x.is_voter(&1));
    assert!(x.is_voter(&2));
    assert!(x.is_voter(&3));
    assert!(x.is_voter(&4));
    assert!(x.is_voter(&5));
    assert!(!x.is_voter(&6));

    Ok(())
}
