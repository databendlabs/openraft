use maplit::btreemap;
use maplit::btreeset;

use super::ChangeMembers;
use crate::Membership;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipEdit;

#[test]
fn nested_batch_applies_edits_in_order_before_finishing_target() {
    let add = ChangeMembers::AddNodes(btreemap! {1 => 10, 2 => 20});
    let voters = ChangeMembers::ReplaceAllVoters(btreeset! {1, 2});
    let update = ChangeMembers::SetNodes(btreemap! {2 => 21});
    let nested = ChangeMembers::Batch(vec![voters, update]);
    let remove = ChangeMembers::RemoveVoters(btreeset! {1});
    let edit = ChangeMembers::Batch(vec![add, nested, remove]);
    let membership = Membership::<u64, u64>::default();
    let actual = edit.apply(&membership);
    let expected = Membership::new(vec![btreeset! {2}], btreemap! {1 => 10, 2 => 21});
    let expected = expected.unwrap();
    let expected = Ok(expected);
    assert_eq!(expected, actual);
}

#[test]
fn node_edit_without_config_returns_empty_membership() {
    let edit = ChangeMembers::AddNodes(btreemap! {1 => 10});
    let membership = Membership::<u64, u64>::default();
    let actual = edit.apply(&membership);
    let expected = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(expected, actual);
}

#[test]
fn test_membership_change() -> anyhow::Result<()> {
    let m = || Membership::<u64, ()> {
        configs: vec![btreeset! {1,2}],
        nodes: btreemap! {1=>(),2=>(),3=>()},
    };

    // Add: no such learner; `next_membership()` rejects this result
    {
        let membership = m();
        let change = ChangeMembers::AddVoterIds(btreeset! {4});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2,4}],
                nodes: btreemap! {1=>(),2=>(),3=>()}
            }),
            res
        );
    }

    // Add: ok
    {
        let membership = m();
        let change = ChangeMembers::AddVoterIds(btreeset! {3});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2,3}],
                nodes: btreemap! {1=>(),2=>(),3=>()}
            }),
            res
        );
    }

    // AddVoters
    {
        let membership = m();
        let change = ChangeMembers::AddVoters(btreemap! {5=>()});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2,5}],
                nodes: btreemap! {1=>(),2=>(),3=>(),5=>()}
            }),
            res
        );
    }

    // Remove: no such voter
    {
        let membership = m();
        let change = ChangeMembers::RemoveVoters(btreeset! {5});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>(),2=>(),3=>()}
            }),
            res
        );
    }

    // Remove: become empty; `next_membership()` rejects this result
    {
        let membership = m();
        let change = ChangeMembers::RemoveVoters(btreeset! {1,2});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {}],
                nodes: btreemap! {1=>(),2=>(),3=>()}
            }),
            res
        );
    }

    // Remove: OK retain
    {
        let membership = m();
        let change = ChangeMembers::RemoveVoters(btreeset! {1});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {2}],
                nodes: btreemap! {1=>(),2=>(),3=>()}
            }),
            res
        );
    }

    // Remove: OK, not retain; learner not removed
    {
        let membership = m();
        let change = ChangeMembers::RemoveVoters(btreeset! {1});
        let goal = change.apply(&membership)?;
        let res = m().drop_voters_demoted_to_learners(goal);
        assert_eq!(
            Membership::<u64, ()> {
                configs: vec![btreeset! {2}],
                nodes: btreemap! {2=>(),3=>()}
            },
            res
        );
    }

    // Remove: OK, not retain; learner removed
    {
        let mem = Membership::<u64, ()> {
            configs: vec![btreeset! {1,2}, btreeset! {2}],
            nodes: btreemap! {1=>(),2=>(),3=>()},
        };
        let change = ChangeMembers::RemoveVoters(btreeset! {1});
        let goal = change.apply(&mem)?;
        let res = mem.drop_voters_demoted_to_learners(goal);
        assert_eq!(
            Membership::<u64, ()> {
                configs: vec![btreeset! {2}],
                nodes: btreemap! {2=>(),3=>()}
            },
            res
        );
    }

    // Replace:
    {
        let membership = m();
        let change = ChangeMembers::ReplaceAllVoters(btreeset! {2});
        let goal = change.apply(&membership)?;
        let res = m().drop_voters_demoted_to_learners(goal);
        assert_eq!(
            Membership::<u64, ()> {
                configs: vec![btreeset! {2}],
                nodes: btreemap! {2=>(),3=>()}
            },
            res
        );
    }

    // AddNodes: existent voter
    {
        let membership = m();
        let change = ChangeMembers::AddNodes(btreemap! {2=>()});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>(),2=>(),3=>()}
            }),
            res
        );
    }

    // AddNodes: existent learner
    {
        let membership = m();
        let change = ChangeMembers::AddNodes(btreemap! {3=>()});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>(),2=>(),3=>()}
            }),
            res
        );
    }

    // AddNodes: Ok
    {
        let membership = m();
        let change = ChangeMembers::AddNodes(btreemap! {4=>()});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>(),2=>(),3=>(), 4=>()}
            }),
            res
        );
    }

    // SetNodes: Ok
    {
        let m = || Membership::<u64, u64> {
            configs: vec![btreeset! {1,2}],
            nodes: btreemap! {1=>1,2=>2,3=>3},
        };

        let membership = m();
        let change = ChangeMembers::SetNodes(btreemap! {3=>30, 4=>40});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, u64> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>1,2=>2,3=>30, 4=>40}
            }),
            res
        );
    }

    // RemoveNodes: cannot remove node for voter; `next_membership()` rejects this result
    {
        let membership = m();
        let change = ChangeMembers::RemoveNodes(btreeset! {2});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>(),3=>()}
            }),
            res
        );
    }

    // RemoveNodes: Ok
    {
        let membership = m();
        let change = ChangeMembers::RemoveNodes(btreeset! {3});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>(),2=>()}
            }),
            res
        );
    }

    // ReplaceAllNodes: Ok
    {
        let membership = m();
        let change = ChangeMembers::ReplaceAllNodes(btreemap! {1=>(),2=>(),4=>()});
        let res = change.apply(&membership);
        assert_eq!(
            Ok(Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>(),2=>(),4=>()}
            }),
            res
        );
    }

    Ok(())
}

/// Test membership change described by a batch operation.
///
/// The batch operations add one voter and remove another.
#[test]
fn test_membership_change_batch() -> anyhow::Result<()> {
    let m = || Membership::<u64, ()> {
        configs: vec![btreeset! {1,2,3}],
        nodes: btreemap! {1=>(),2=>(),3=>()},
    };

    let rm_2_add_5 = || {
        ChangeMembers::Batch(vec![
            ChangeMembers::RemoveVoters(btreeset! {2}),
            ChangeMembers::AddVoters(btreemap! {5=>()}),
        ])
    };

    let membership = m();
    let change = rm_2_add_5();
    let goal = change.apply(&membership)?;

    assert_eq!(goal, Membership::<u64, ()> {
        configs: vec![btreeset! {1,3,5}],
        nodes: btreemap! {1=>(),2=>(),3=>(),5=>()}
    });

    Ok(())
}
