use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;
use maplit::btreeset;
use openraft::Node;
use openraft::errors::EmptyMembership;
use openraft::errors::MembershipError;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::IntoNodes;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;

use crate::GridChange;
use crate::GridMembership;
use crate::errors::InvalidGrid;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::testing::accepted;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::holding_one_of;
use crate::quorum::testing::meeting_each_of;
use crate::quorum::testing::small_grids;
use crate::testing::assert_apply_checks_roles;
use crate::testing::assert_change_through_joint_keeps_node_metadata;
use crate::testing::assert_claims_hold;
use crate::testing::assert_joint_keeps_old_roles_and_target_restores_new_roles;
use crate::testing::assert_quorum_sets_are_valid;
use crate::testing::assert_role_and_quorum_change_through_joint;
use crate::testing::assert_with_roles;
use crate::testing::voter_role;

type Membership = GridMembership<u64, ()>;

/// Returns a membership whose only config places its voters in `grid`, with a `()` node for each
/// voter.
fn grid(grid: Vec<Vec<u64>>) -> Membership {
    let cells = grid.iter().flatten();
    let voters: BTreeSet<u64> = cells.copied().collect();
    GridMembership::new(grid, voters).unwrap()
}

/// Returns a membership whose only config places `voters` in one row, with each node of `nodes`.
fn one_row<N, T>(voters: BTreeSet<u64>, nodes: T) -> GridMembership<u64, N>
where
    N: Node,
    T: IntoNodes<u64, N>,
{
    let row = voters.into_iter().collect();
    GridMembership::new(vec![row], nodes).unwrap()
}

#[test]
fn test_new() {
    let membership = Membership::new(vec![vec![1, 2], vec![3, 4]], btreeset! {1, 2, 3, 4, 5}).unwrap();
    let want = "{configs: [Config { read: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: FullRow }, \
                write: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: OnePerRow }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs)), 5: ((), NodeRole(ReceiveLogs))} }]}";
    let got = membership.to_string();
    assert_eq!(want, got);

    let cases = [
        (vec![], InvalidGrid::NoRow),
        (vec![vec![]], InvalidGrid::NoColumn),
        (vec![vec![1, 2], vec![3]], InvalidGrid::RaggedRow {
            row: 1,
            len: 1,
            column_count: 2,
        }),
        (vec![vec![1, 2], vec![3, 1]], InvalidGrid::DuplicateVoter { voter: 1 }),
    ];
    for (cells, invalid) in cases {
        let res = Membership::new(cells, btreeset! {1, 2, 3});
        assert_eq!(Err(MembershipError::UserDefined(invalid)), res);
    }

    // Voter 4 has no node.
    let res = Membership::new(vec![vec![1, 2], vec![3, 4]], btreeset! {1, 2, 3});
    let want = Err(MembershipError::NodeNotFound(NodeNotFound::new(4, Operation::None)));
    assert_eq!(want, res);
}

#[test]
fn test_quorum_sets_accept_exactly_the_sets_holding_a_full_row_or_meeting_every_row() {
    let membership = grid(vec![vec![1, 2, 3], vec![4, 5, 6]]);
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4, 5, 6});

    let read_accepted = accepted(&membership.read_quorum_set(), &subsets);
    let rows = [btreeset! {1, 2, 3}, btreeset! {4, 5, 6}];
    let want = holding_one_of(&rows, &subsets);
    let count = want.len();
    assert_eq!(15, count);
    assert_eq!(want, read_accepted);

    // A write quorum holds a voter of each row from any column, such as {1,5} or {3,4}, but not
    // {1,2,3}, which misses row {4,5,6}.
    let write_accepted = accepted(&membership.write_quorum_set(), &subsets);
    let want = meeting_each_of(&rows, &subsets);
    let count = want.len();
    assert_eq!(49, count);
    assert_eq!(want, write_accepted);
}

#[test]
fn test_get_node_reports_each_voter_as_read_and_write_voter() {
    let membership = Membership::new(vec![vec![1, 2], vec![3, 4]], btreeset! {1, 2, 3, 4, 5}).unwrap();

    for node_id in [1, 2, 3, 4] {
        let got = membership.get_node(&node_id);
        let want = Some(NodeRef {
            node: &(),
            role: voter_role(),
            is_read_voter: true,
            is_write_voter: true,
        });
        assert_eq!(want, got, "node {node_id}");
    }

    let learner = membership.get_node(&5);
    let want = Some(NodeRef {
        node: &(),
        role: NodeRole::ReceiveLogs,
        is_read_voter: false,
        is_write_voter: false,
    });
    assert_eq!(want, learner);

    let absent = membership.get_node(&6);
    assert_eq!(None, absent);

    let ids: Vec<u64> = membership.node_ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5], ids);
    let read_quorum_set = membership.read_quorum_set();
    let read_ids: Vec<u64> = read_quorum_set.ids().collect();
    assert_eq!(vec![1, 2, 3, 4], read_ids);
    let write_quorum_set = membership.write_quorum_set();
    let write_ids: Vec<u64> = write_quorum_set.ids().collect();
    assert_eq!(vec![1, 2, 3, 4], write_ids);
}

#[test]
fn test_from_node_map_fills_the_most_square_grid() {
    let cases = [
        (1, vec![vec![1]]),
        (2, vec![vec![1, 2]]),
        (4, vec![vec![1, 2], vec![3, 4]]),
        (5, vec![vec![1, 2, 3, 4, 5]]),
        (6, vec![vec![1, 2, 3], vec![4, 5, 6]]),
        (8, vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]),
        (9, vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]]),
        (12, vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![9, 10, 11, 12]]),
    ];
    for (node_count, cells) in cases {
        let nodes: BTreeMap<u64, ()> = (1..=node_count).map(|node_id| (node_id, ())).collect();
        let membership = Membership::from(nodes);
        let want = grid(cells);
        assert_eq!(want, membership, "{node_count} nodes");
    }

    // The cells take the IDs in ascending order.
    let nodes = btreemap! {9 => (), 3 => (), 7 => (), 1 => ()};
    let membership = Membership::from(nodes);
    let want = grid(vec![vec![1, 3], vec![7, 9]]);
    assert_eq!(want, membership);

    let empty = Membership::from(BTreeMap::new());
    let want = Membership::default();
    assert_eq!(want, empty);
    let got = empty.to_string();
    assert_eq!("{configs: []}", got);
}

#[test]
fn test_direct_transitions_need_the_same_grid() {
    let square = grid(vec![vec![1, 2], vec![3, 4]]);

    let same = square.supports_direct_transition_to(&square);
    assert!(same);

    // Swapping the voters of a row keeps the rows, so both rules accept the same sets as before.
    // The grid changes, so the change still goes through a joint.
    let swapped = grid(vec![vec![2, 1], vec![3, 4]]);
    let direct = square.supports_direct_transition_to(&swapped);
    assert!(!direct);

    // Transposing turns the rows into columns: old row {1,2} misses new write quorum {3,4}, which
    // holds a voter of new row {1,3} and of new row {2,4}.
    let transposed = grid(vec![vec![1, 3], vec![2, 4]]);
    let direct = square.supports_direct_transition_to(&transposed);
    assert!(!direct);

    // A third row meets the old rules in one direction only. Every new write quorum holds a voter
    // of each old row, but new row {5,6} misses old write quorum {1,3}.
    let tall = grid(vec![vec![1, 2], vec![3, 4], vec![5, 6]]);
    let old_reads_meet_new_writes = square.read_quorum_set().intersects_with(&tall.write_quorum_set());
    assert_eq!(Some(true), old_reads_meet_new_writes);
    let old_writes_meet_new_reads = square.write_quorum_set().intersects_with(&tall.read_quorum_set());
    assert_eq!(Some(false), old_writes_meet_new_reads);
    let grow = square.supports_direct_transition_to(&tall);
    assert!(!grow);
    let shrink = tall.supports_direct_transition_to(&square);
    assert!(!shrink);

    // A joint goes directly to its target, and adding a learner changes no quorum.
    let joint = square.joint_with(&transposed);
    let to_target = joint.supports_direct_transition_to(&transposed);
    assert!(to_target);
    let add_learner = <GridChange<u64, ()> as RaftMembershipEdit<Membership>>::add_learner(9, ());
    let with_learner = add_learner.apply(&square, true).unwrap();
    let to_learner = square.supports_direct_transition_to(&with_learner);
    assert!(to_learner);
}

#[test]
fn test_small_memberships_go_directly_only_with_the_same_grid() {
    let grids = small_grids();
    let mut memberships = vec![];
    for cells in &grids {
        let membership = grid(cells.clone());
        assert_quorum_sets_are_valid(&membership);
        memberships.push(membership);
    }

    // Renaming the voters of both memberships keeps every answer, so the old membership only
    // needs to fill its grid with 1, 2, 3, ... row by row.
    for (cells, old) in grids.iter().zip(&memberships) {
        let filled = cells.concat();
        let ascending: Vec<u64> = (1..=filled.len() as u64).collect();
        if filled != ascending {
            continue;
        }
        for new in &memberships {
            assert_claims_hold(old, new);

            // Both rules of a config hold its grid, so only the same grid shares both rules.
            let same_grid = old.read_quorum_set() == new.read_quorum_set();
            let direct = old.supports_direct_transition_to(new);
            assert_eq!(same_grid, direct, "{old} to {new}");
        }
    }
}

#[test]
fn test_placement_change_goes_through_joint_that_keeps_both_grids() {
    let old = grid(vec![vec![1, 2], vec![3, 4]]);
    let change = GridChange::grid(vec![vec![1, 3], vec![2, 4]]);
    let new = change.apply(&old, true).unwrap();
    let direct = old.supports_direct_transition_to(&new);
    assert!(!direct);

    // A read quorum holds a row of each grid, such as {1,2} and {1,3}, so it takes 3 of the 4
    // voters.
    let joint = old.joint_with(&new);
    let subsets = all_subsets(&btreeset! {1, 2, 3, 4});
    let any_three = [
        btreeset! {1, 2, 3},
        btreeset! {1, 2, 4},
        btreeset! {1, 3, 4},
        btreeset! {2, 3, 4},
    ];
    let want = holding_one_of(&any_three, &subsets);
    let count = want.len();
    assert_eq!(5, count);
    let read_accepted = accepted(&joint.read_quorum_set(), &subsets);
    assert_eq!(want, read_accepted);

    // A write quorum holds a voter of each row of both grids. A diagonal of the old grid, {1,4} or
    // {2,3}, does that with 2 voters.
    let diagonals = [btreeset! {1, 4}, btreeset! {2, 3}];
    let want = holding_one_of(&diagonals, &subsets);
    let count = want.len();
    assert_eq!(7, count);
    let write_accepted = accepted(&joint.write_quorum_set(), &subsets);
    assert_eq!(want, write_accepted);

    let want = "{configs: [Config { read: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: FullRow }, \
                write: GridQuorumSet { grid: [[1, 2], [3, 4]], rule: OnePerRow }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(Campaigner | ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs))} }, Config { read: GridQuorumSet { grid: [[1, 3], [2, \
                4]], rule: FullRow }, write: GridQuorumSet { grid: [[1, 3], [2, 4]], rule: OnePerRow }, \
                nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), 2: ((), NodeRole(Campaigner | ReceiveLogs)), \
                3: ((), NodeRole(Campaigner | ReceiveLogs)), 4: ((), NodeRole(Campaigner | ReceiveLogs))} }]}";
    let got = joint.to_string();
    assert_eq!(want, got);

    let change = <GridChange<u64, ()> as RaftMembershipEdit<Membership>>::make_uniform();
    let target = change.apply(&joint, true).unwrap();
    assert_eq!(new, target);
    let direct = joint.supports_direct_transition_to(&target);
    assert!(direct);
}

#[test]
fn test_make_uniform_selects_target() {
    let old = grid(vec![vec![1, 2], vec![3, 4]]);

    // Voters 3 and 4 of the old grid stay as learners of the target only when the target holds
    // them: OpenRaft removes them from the target unless the change retains removed voters.
    let with_learners = Membership::new(vec![vec![1, 2]], btreeset! {1, 2, 3, 4}).unwrap();
    let without_learners = grid(vec![vec![1, 2]]);
    for new in [with_learners, without_learners] {
        let joint = old.joint_with(&new);

        let change = <GridChange<u64, ()> as RaftMembershipEdit<Membership>>::make_uniform();
        let target = change.apply(&joint, true).unwrap();
        assert_eq!(new, target);
        let direct = joint.supports_direct_transition_to(&target);
        assert!(direct);
    }
}

#[test]
fn test_apply() {
    let nodes = btreemap! {
        1 => "a".to_string(),
        2 => "b".to_string(),
        3 => "c".to_string(),
        4 => "d".to_string(),
    };
    let square = vec![vec![1, 2], vec![3, 4]];
    let membership = GridMembership::new(square.clone(), nodes.clone()).unwrap();

    // A new placement keeps the nodes.
    let transposed = vec![vec![1, 3], vec![2, 4]];
    let change = GridChange::grid(transposed.clone());
    let res = change.apply(&membership, true);
    let want = GridMembership::new(transposed, nodes.clone());
    assert_eq!(want, res);

    // New voters 5 and 6 join with their nodes as a third column.
    let wider = vec![vec![1, 2, 5], vec![3, 4, 6]];
    let change = GridChange {
        add_nodes: btreemap! {5 => "e".to_string(), 6 => "f".to_string()},
        grid: Some(wider.clone()),
        ..GridChange::default()
    };
    let res = change.apply(&membership, true);
    let mut want_nodes = nodes.clone();
    want_nodes.insert(5, "e".to_string());
    want_nodes.insert(6, "f".to_string());
    let want = GridMembership::new(wider, want_nodes);
    assert_eq!(want, res);

    // Voters 3 and 4 leave the grid and are removed in one change.
    let change = GridChange {
        grid: Some(vec![vec![1, 2]]),
        remove_nodes: btreeset! {3, 4},
        ..GridChange::default()
    };
    let res = change.apply(&membership, true);
    let want_nodes = btreemap! {1 => "a".to_string(), 2 => "b".to_string()};
    let want = GridMembership::new(vec![vec![1, 2]], want_nodes);
    assert_eq!(want, res);

    // Adding nodes keeps the metadata of a known node; removing a learner drops it.
    let change = GridChange {
        add_nodes: btreemap! {1 => "z".to_string(), 5 => "e".to_string()},
        ..GridChange::default()
    };
    let with_five = change.apply(&membership, true).unwrap();
    let mut want_nodes = nodes.clone();
    want_nodes.insert(5, "e".to_string());
    let want = GridMembership::new(square.clone(), want_nodes).unwrap();
    assert_eq!(want, with_five);

    let change = GridChange {
        remove_nodes: btreeset! {5},
        ..GridChange::default()
    };
    let res = change.apply(&with_five, true);
    let want = Ok(membership.clone());
    assert_eq!(want, res);

    // Removing a voter fails, and so does placing a voter without a node, because every voter
    // needs a node.
    let change = GridChange {
        remove_nodes: btreeset! {3},
        ..GridChange::default()
    };
    let res = change.apply(&membership, true);
    let want = Err(MembershipError::from(NodeNotFound::new(3, Operation::None)));
    assert_eq!(want, res);

    let change = GridChange::grid(vec![vec![1, 2], vec![3, 7]]);
    let res = change.apply(&membership, true);
    let want = Err(MembershipError::from(NodeNotFound::new(7, Operation::None)));
    assert_eq!(want, res);

    // Invalid grids are the edit's own errors.
    let change = GridChange::grid(vec![vec![1, 2], vec![3]]);
    let res = change.apply(&membership, true);
    let ragged = InvalidGrid::RaggedRow {
        row: 1,
        len: 1,
        column_count: 2,
    };
    assert_eq!(Err(MembershipError::UserDefined(ragged)), res);

    let change = GridChange::grid(vec![vec![1, 2], vec![2, 3]]);
    let res = change.apply(&membership, true);
    let duplicate = InvalidGrid::DuplicateVoter { voter: 2 };
    assert_eq!(Err(MembershipError::UserDefined(duplicate)), res);

    // `add_learner()` never replaces the node of a known ID.
    type Change = GridChange<u64, String>;
    type Target = GridMembership<u64, String>;
    let change = <Change as RaftMembershipEdit<Target>>::add_learner(2, "z".to_string());
    let res = change.apply(&membership, true);
    let want = Ok(membership.clone());
    assert_eq!(want, res);

    // A membership with no config has no target to change.
    let empty = Target::default();
    let change = GridChange::grid(square);
    let res = change.apply(&empty, true);
    let want = Err(MembershipError::EmptyMembership(EmptyMembership {}));
    assert_eq!(want, res);
}

#[test]
fn test_joint_keeps_old_roles_and_target_restores_new_roles() {
    let old = Membership::new(vec![vec![1, 2]], btreeset! {1, 2, 3}).unwrap();

    // The new config stops voter 2 from campaigning.
    let new = old.clone().with_roles(btreemap! {2 => NodeRole::ReceiveLogs}).unwrap();
    let target = assert_joint_keeps_old_roles_and_target_restores_new_roles::<_, GridChange<u64, ()>>(&old, &new, 2);

    let want = "{configs: [Config { read: GridQuorumSet { grid: [[1, 2]], rule: FullRow }, write: GridQuorumSet { \
                grid: [[1, 2]], rule: OnePerRow }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                2: ((), NodeRole(ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs))} }]}";
    let got = target.to_string();
    assert_eq!(want, got);

    // A new grid keeps the role that the last config sets for voter 2.
    let change = GridChange::grid(vec![vec![1], vec![2], vec![3]]);
    let moved = change.apply(&target, true).unwrap();
    let in_moved = moved.get_node(&2).unwrap();
    assert_eq!(NodeRole::ReceiveLogs, in_moved.role);
}

#[test]
fn test_with_roles() {
    let want = "{configs: [Config { read: GridQuorumSet { grid: [[1, 2, 3]], rule: FullRow }, write: GridQuorumSet { \
                grid: [[1, 2, 3]], rule: OnePerRow }, nodes: {1: ((), NodeRole(Campaigner | ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs)), 5: ((), NodeRole(Empty))} }]}";
    assert_with_roles(one_row, Membership::with_roles, want);
}

#[test]
fn test_role_and_quorum_change_through_joint() {
    let want = "{configs: [Config { read: GridQuorumSet { grid: [[2, 3, 4]], rule: FullRow }, write: GridQuorumSet { \
                grid: [[2, 3, 4]], rule: OnePerRow }, nodes: {1: ((), NodeRole(ReceiveLogs)), \
                2: ((), NodeRole(Campaigner | ReceiveLogs)), 3: ((), NodeRole(ReceiveLogs)), \
                4: ((), NodeRole(Campaigner | ReceiveLogs)), 5: ((), NodeRole(Empty))} }]}";
    assert_role_and_quorum_change_through_joint(one_row, Membership::with_roles, want, |voters, roles| {
        let row = voters.into_iter().collect();
        GridChange::grid(vec![row]).with_roles(roles)
    });
}

#[test]
fn test_change_through_joint_keeps_node_metadata() {
    assert_change_through_joint_keeps_node_metadata(one_row, |voters, remove_nodes| {
        let row = voters.into_iter().collect();
        GridChange {
            remove_nodes,
            ..GridChange::grid(vec![row])
        }
    });
}

#[test]
fn test_apply_checks_roles() {
    assert_apply_checks_roles(one_row, Membership::with_roles, |add_nodes, remove_nodes, roles| {
        GridChange {
            add_nodes,
            remove_nodes,
            roles,
            ..GridChange::default()
        }
    });
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let membership = Membership::new(vec![vec![1, 2], vec![3, 4]], btreeset! {1, 2, 3, 4, 5}).unwrap();

    let json = serde_json::to_string(&membership).unwrap();
    let want = r#"{"configs":[{"read":{"grid":[[1,2],[3,4]],"rule":"FullRow"},"write":{"grid":[[1,2],[3,4]],"rule":"OnePerRow"},"nodes":{"1":[null,3],"2":[null,3],"3":[null,3],"4":[null,3],"5":[null,2]}}]}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<Membership>(&json).unwrap();
    assert_eq!(membership, decoded);

    let empty = r#"{"configs":[]}"#;
    let decoded = serde_json::from_str::<Membership>(empty).unwrap();
    let want = Membership::default();
    assert_eq!(want, decoded);
}

#[cfg(feature = "serde")]
#[test]
fn test_roundtrip_keeps_joint_configs_and_roles() {
    let old = grid(vec![vec![1, 2], vec![3, 4]]);

    // The new grid transposes the old one and stops voter 3 from campaigning.
    let transposed = grid(vec![vec![1, 3], vec![2, 4]]);
    let roles = btreemap! {3 => NodeRole::ReceiveLogs};
    let new = transposed.with_roles(roles).unwrap();
    let joint = old.joint_with(&new);

    let json = serde_json::to_string(&joint).unwrap();
    let want = r#"{"configs":[{"read":{"grid":[[1,2],[3,4]],"rule":"FullRow"},"write":{"grid":[[1,2],[3,4]],"rule":"OnePerRow"},"nodes":{"1":[null,3],"2":[null,3],"3":[null,3],"4":[null,3]}},{"read":{"grid":[[1,3],[2,4]],"rule":"FullRow"},"write":{"grid":[[1,3],[2,4]],"rule":"OnePerRow"},"nodes":{"1":[null,3],"2":[null,3],"3":[null,2],"4":[null,3]}}]}"#;
    assert_eq!(want, json);
    let decoded = serde_json::from_str::<Membership>(&json).unwrap();
    assert_eq!(joint, decoded);

    let binary = bincode::serialize(&joint).unwrap();
    let decoded = bincode::deserialize::<Membership>(&binary).unwrap();
    assert_eq!(joint, decoded);
}
