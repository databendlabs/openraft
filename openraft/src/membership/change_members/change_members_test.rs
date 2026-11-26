use maplit::btreemap;
use maplit::btreeset;

use super::ChangeMembers;
use crate::Membership;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::membership::RaftMembershipEdit;
use crate::membership::RaftMembershipEditOps;

#[test]
fn nested_batch_applies_edits_in_order_before_finishing_target() {
    let add = ChangeMembers::AddNodes(btreemap! {1 => 10, 2 => 20});
    let voters = ChangeMembers::ReplaceAllVoters(btreeset! {1, 2});
    let update = ChangeMembers::SetNodes(btreemap! {2 => 21});
    let nested = ChangeMembers::Batch(vec![voters, update]);
    let remove = ChangeMembers::RemoveVoters(btreeset! {1});
    let edit = ChangeMembers::Batch(vec![add, nested, remove]);
    let membership = Membership::<u64, u64>::default();
    let actual = edit.apply(&membership, true);
    let expected = Membership::new(vec![btreeset! {2}], btreemap! {1 => 10, 2 => 21});
    let expected = expected.unwrap();
    let expected = Ok(expected);
    assert_eq!(expected, actual);
}

#[test]
fn node_edit_without_config_returns_empty_membership() {
    let edit = ChangeMembers::AddNodes(btreemap! {1 => 10});
    let membership = Membership::<u64, u64>::default();
    let actual = edit.apply(&membership, true);
    let expected = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(expected, actual);
}

#[test]
fn batch_retention_uses_the_original_and_final_voters() -> anyhow::Result<()> {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40};
    let membership = Membership::new(vec![btreeset! {1, 2}], nodes)?;
    let remove = ChangeMembers::RemoveVoters(btreeset! {1, 2});
    let promote = ChangeMembers::AddVoterIds(btreeset! {2, 3});
    let demote = ChangeMembers::RemoveVoters(btreeset! {3});
    let nested = ChangeMembers::Batch(vec![promote, demote]);
    let change = ChangeMembers::Batch(vec![remove, nested]);

    let result = change.apply(&membership, false)?;

    let expected = Membership::new(vec![btreeset! {2}], btreemap! {2 => 20, 3 => 30, 4 => 40})?;
    assert_eq!(expected, result);
    Ok(())
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, false)?;
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
        let res = change.apply(&mem, false)?;
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
        let res = change.apply(&membership, false)?;
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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
        let res = change.apply(&membership, true);
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

#[test]
fn edit_of_joint_respects_retention_and_preserves_learner_metadata() -> anyhow::Result<()> {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50, 6 => 60};
    let joint = Membership::<u64, u64>::new(vec![btreeset! {1, 2, 3}, btreeset! {3, 4, 5}], nodes.clone())?;
    for retain in [true, false] {
        let change = ChangeMembers::<u64, u64>::RemoveVoters(btreeset! {3});
        let result = change.apply(&joint, retain)?;
        let target_nodes = if retain {
            nodes.clone()
        } else {
            btreemap! {4 => 40, 5 => 50, 6 => 60}
        };
        let expected = Membership::new(vec![btreeset! {4, 5}], target_nodes)?;
        assert_eq!(expected, result);
    }
    Ok(())
}

/// The learner edit keeps every voter set and the node of a known ID, whatever `retain` is.
#[test]
fn learner_edit_keeps_every_voter_set() -> anyhow::Result<()> {
    let sets = [btreeset! {1, 2}, btreeset! {2, 3}, btreeset! {3, 4}];
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40, 5 => 50};

    for set_count in 1..=sets.len() {
        let configs = sets[sets.len() - set_count..].to_vec();
        let membership = Membership::new(configs.clone(), nodes.clone())?;

        for retain in [false, true] {
            tracing::info!(set_count, retain, "--- add node 6 as a learner");
            {
                let mut want_nodes = nodes.clone();
                want_nodes.insert(6, 60);
                let want = Membership::new(configs.clone(), want_nodes)?;
                let got = ChangeMembers::add_learner(6, 60).apply(&membership, retain)?;
                assert_eq!(want, got);
            }

            tracing::info!(set_count, retain, "--- re-add known nodes, voters or learners");
            {
                for node_id in [1, 2, 4, 5] {
                    let got = ChangeMembers::add_learner(node_id, 99).apply(&membership, retain)?;
                    assert_eq!(membership, got, "re-add {node_id}");
                }
            }
        }
    }
    Ok(())
}

/// A node edit, also in a nested batch, keeps every voter set of a joint membership and demotes
/// no voter, whatever `retain` is.
#[test]
fn node_edits_keep_joint_voter_sets() -> anyhow::Result<()> {
    let configs = vec![btreeset! {1, 2}, btreeset! {2, 3}];
    let joint = Membership::new(configs.clone(), btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 40})?;
    let batch = ChangeMembers::Batch(vec![
        ChangeMembers::RemoveNodes(btreeset! {4}),
        ChangeMembers::Batch(vec![ChangeMembers::AddNodes(btreemap! {4 => 41, 5 => 50})]),
        ChangeMembers::SetNodes(btreemap! {5 => 51}),
    ]);
    let cases = [
        (
            ChangeMembers::SetNodes(btreemap! {1 => 11, 5 => 50}),
            btreemap! {1 => 11, 2 => 20, 3 => 30, 4 => 40, 5 => 50},
        ),
        (
            ChangeMembers::RemoveNodes(btreeset! {4}),
            btreemap! {1 => 10, 2 => 20, 3 => 30},
        ),
        (
            ChangeMembers::ReplaceAllNodes(btreemap! {1 => 11, 2 => 21, 3 => 31}),
            btreemap! {1 => 11, 2 => 21, 3 => 31},
        ),
        (batch, btreemap! {1 => 10, 2 => 20, 3 => 30, 4 => 41, 5 => 51}),
    ];

    for retain in [false, true] {
        for (edit, nodes) in cases.clone() {
            let name = edit.to_string();
            let want = Membership::new(configs.clone(), nodes)?;
            assert_eq!(want, edit.apply(&joint, retain)?, "{name}, retain={retain}");
        }
    }
    Ok(())
}

/// A voter edit leaves a joint membership: after it, the target holds only the voter set that the
/// edit computes from the last set, and `retain` decides whether demoted voter 1 keeps its node.
#[test]
fn voter_edit_in_batch_leaves_joint_membership() -> anyhow::Result<()> {
    let nodes = btreemap! {1 => 10, 2 => 20, 3 => 30};
    let joint = Membership::new(vec![btreeset! {1, 2}, btreeset! {2, 3}], nodes)?;

    for retain in [false, true] {
        let edit = ChangeMembers::Batch(vec![
            ChangeMembers::add_learner(4, 40),
            ChangeMembers::AddVoterIds(btreeset! {}),
        ]);
        let mut nodes = btreemap! {2 => 20, 3 => 30, 4 => 40};
        if retain {
            nodes.insert(1, 10);
        }
        let want = Membership::new(vec![btreeset! {2, 3}], nodes)?;
        assert_eq!(want, edit.apply(&joint, retain)?, "retain={retain}");
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
    let goal = change.apply(&membership, true)?;

    assert_eq!(goal, Membership::<u64, ()> {
        configs: vec![btreeset! {1,3,5}],
        nodes: btreemap! {1=>(),2=>(),3=>(),5=>()}
    });

    Ok(())
}
