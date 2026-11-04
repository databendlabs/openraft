use std::collections::BTreeSet;

use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::errors::InvalidGrid;
use crate::quorum::ExplicitQuorumSet;
use crate::quorum::GridQuorumSet;
use crate::quorum::GridRule;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::testing::accepted;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::arrangements;
use crate::quorum::testing::assert_quorum_set_contract;
use crate::quorum::testing::holding_one_of;
use crate::quorum::testing::meeting_each_of;
use crate::quorum::testing::small_grids;

/// Returns the `rule` over the valid `grid`.
fn grid(grid: Vec<Vec<u64>>, rule: GridRule) -> GridQuorumSet<u64> {
    GridQuorumSet::new(grid, rule).expect("the grid is valid")
}

#[test]
fn test_new_rejects_invalid_grids() {
    let cases = [
        (vec![], InvalidGrid::NoRow),
        (vec![vec![]], InvalidGrid::NoColumn),
        (vec![vec![], vec![1]], InvalidGrid::NoColumn),
        (vec![vec![1, 2], vec![3]], InvalidGrid::RaggedRow {
            row: 1,
            len: 1,
            column_count: 2,
        }),
        (vec![vec![1], vec![2], vec![]], InvalidGrid::RaggedRow {
            row: 2,
            len: 0,
            column_count: 1,
        }),
        (vec![vec![1, 1]], InvalidGrid::DuplicateVoter { voter: 1 }),
        // The cell of row 1, column 0 is the first to repeat a voter.
        (vec![vec![1, 2], vec![2, 1]], InvalidGrid::DuplicateVoter { voter: 2 }),
        // The shape is checked before the voters.
        (vec![vec![1, 1], vec![2]], InvalidGrid::RaggedRow {
            row: 1,
            len: 1,
            column_count: 2,
        }),
    ];

    for (cells, invalid) in cases {
        for rule in [GridRule::FullRow, GridRule::OnePerRow] {
            let res = GridQuorumSet::new(cells.clone(), rule);
            let want = Err(invalid.clone());
            assert_eq!(want, res, "grid={cells:?}, rule={rule:?}");
        }
    }
}

#[test]
fn test_rules_accept_the_sets_of_equal_explicit_lists() {
    // Each case lists a grid, its rows, and its sets of one voter of each row.
    let cases = [
        // A square grid: a OnePerRow quorum can take its voters from different columns.
        (
            vec![vec![1, 2], vec![3, 4]],
            vec![btreeset! {1, 2}, btreeset! {3, 4}],
            vec![btreeset! {1, 3}, btreeset! {1, 4}, btreeset! {2, 3}, btreeset! {2, 4}],
        ),
        // One row: the FullRow rule needs every voter, and the OnePerRow rule any one.
        (vec![vec![1, 2, 3]], vec![btreeset! {1, 2, 3}], vec![
            btreeset! {1},
            btreeset! {2},
            btreeset! {3},
        ]),
        // One column.
        (
            vec![vec![1], vec![2], vec![3]],
            vec![btreeset! {1}, btreeset! {2}, btreeset! {3}],
            vec![btreeset! {1, 2, 3}],
        ),
        // A rectangle whose cells are not in ID order.
        (
            vec![vec![4, 1, 6], vec![2, 5, 3]],
            vec![btreeset! {1, 4, 6}, btreeset! {2, 3, 5}],
            vec![
                btreeset! {4, 2},
                btreeset! {4, 5},
                btreeset! {4, 3},
                btreeset! {1, 2},
                btreeset! {1, 5},
                btreeset! {1, 3},
                btreeset! {6, 2},
                btreeset! {6, 5},
                btreeset! {6, 3},
            ],
        ),
    ];

    for (cells, rows, one_of_each_row) in cases {
        let voters = cells.iter().flatten();
        let ids: BTreeSet<u64> = voters.copied().collect();
        let subsets = all_subsets(&ids);

        let full_row = grid(cells.clone(), GridRule::FullRow);
        let explicit_rows = ExplicitQuorumSet::new(rows).unwrap();
        let got = accepted(&full_row, &subsets);
        let want = accepted(&explicit_rows, &subsets);
        assert_eq!(want, got, "FullRow of {cells:?}");

        let one_per_row = grid(cells.clone(), GridRule::OnePerRow);
        let explicit_one_of_each_row = ExplicitQuorumSet::new(one_of_each_row).unwrap();
        let got = accepted(&one_per_row, &subsets);
        let want = accepted(&explicit_one_of_each_row, &subsets);
        assert_eq!(want, got, "OnePerRow of {cells:?}");
    }
}

#[test]
fn test_small_rules_accept_exactly_the_sets_holding_a_full_row_or_meeting_every_row() {
    for cells in small_grids() {
        let mut ids = BTreeSet::new();
        let mut rows = vec![];
        for row in &cells {
            let row_ids: BTreeSet<u64> = row.iter().copied().collect();
            ids.extend(&row_ids);
            rows.push(row_ids);
        }
        let subsets = all_subsets(&ids);

        let full_row = grid(cells.clone(), GridRule::FullRow);
        assert_quorum_set_contract(&full_row);
        let got = accepted(&full_row, &subsets);
        let want = holding_one_of(&rows, &subsets);
        assert_eq!(want, got, "FullRow of {cells:?}");

        let one_per_row = grid(cells.clone(), GridRule::OnePerRow);
        assert_quorum_set_contract(&one_per_row);
        let got = accepted(&one_per_row, &subsets);
        let want = meeting_each_of(&rows, &subsets);
        assert_eq!(want, got, "OnePerRow of {cells:?}");
    }
}

#[test]
fn test_rules_hold_their_grid() {
    let cells = vec![vec![4, 1, 6], vec![2, 5, 3]];
    let full_row = GridQuorumSet::new(cells.clone(), GridRule::FullRow).unwrap();
    let one_per_row = GridQuorumSet::new(cells.clone(), GridRule::OnePerRow).unwrap();

    let full_row_grid = full_row.grid();
    assert_eq!(cells, full_row_grid);
    let one_per_row_grid = one_per_row.grid();
    assert_eq!(cells, one_per_row_grid);
    let full_row_rule = full_row.rule();
    assert_eq!(GridRule::FullRow, full_row_rule);
    let one_per_row_rule = one_per_row.rule();
    assert_eq!(GridRule::OnePerRow, one_per_row_rule);

    let full_row_ids: Vec<u64> = full_row.ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5, 6], full_row_ids);
    let one_per_row_ids: Vec<u64> = one_per_row.ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5, 6], one_per_row_ids);
}

#[test]
fn test_is_quorum_ignores_unknown_and_repeated_ids() {
    let rows = grid(vec![vec![1, 2], vec![3, 4]], GridRule::FullRow);

    let accepts = rows.is_quorum([2, 9, 2, 1].iter());
    assert!(accepts);
    let accepts = rows.is_quorum([1, 1, 3, 9].iter());
    assert!(!accepts);
    let accepts = rows.is_quorum([].iter());
    assert!(!accepts);
}

#[test]
fn test_full_row_rule_and_one_per_row_rule_of_one_grid_intersect() {
    let grids = [
        vec![vec![1]],
        vec![vec![1, 2, 3]],
        vec![vec![1], vec![2], vec![3]],
        vec![vec![1, 2], vec![3, 4]],
        vec![vec![4, 1, 6], vec![2, 5, 3]],
    ];

    for cells in grids {
        let full_row = grid(cells.clone(), GridRule::FullRow);
        let one_per_row = grid(cells.clone(), GridRule::OnePerRow);

        let full_row_meets_one_per_row = full_row.intersects_with(&one_per_row);
        assert_eq!(Some(true), full_row_meets_one_per_row, "{cells:?}");
        let one_per_row_meets_full_row = one_per_row.intersects_with(&full_row);
        assert_eq!(Some(true), one_per_row_meets_full_row, "{cells:?}");

        // Two rows share no voter. Two OnePerRow rules get no answer.
        let has_one_row = cells.len() == 1;
        let full_row_meets_full_row = full_row.intersects_with(&full_row);
        assert_eq!(Some(has_one_row), full_row_meets_full_row, "{cells:?}");
        let one_per_row_meets_one_per_row = one_per_row.intersects_with(&one_per_row);
        assert_eq!(None, one_per_row_meets_one_per_row, "{cells:?}");
    }
}

#[test]
fn test_intersection_is_exact_with_a_full_row_rule() {
    // Every grid of at most 4 cells, which holds distinct IDs of 1 to 4.
    let shapes = [(1, 1), (1, 2), (2, 1), (1, 3), (3, 1), (1, 4), (4, 1), (2, 2)];
    let mut rules = vec![];
    for (row_count, column_count) in shapes {
        for cells in arrangements(&[1, 2, 3, 4], row_count * column_count) {
            let rows = cells.chunks(column_count);
            let cells: Vec<Vec<u64>> = rows.map(|row| row.to_vec()).collect();
            rules.push(grid(cells.clone(), GridRule::FullRow));
            rules.push(grid(cells, GridRule::OnePerRow));
        }
    }
    let rule_count = rules.len();
    assert_eq!(296, rule_count);

    // Renaming the IDs of both rules keeps both answers, so the first rule only needs to fill its
    // cells with 1, 2, 3, ... row by row.
    for a in &rules {
        let cells = a.grid().concat();
        let ascending: Vec<u64> = (1..=cells.len() as u64).collect();
        if cells != ascending {
            continue;
        }
        for b in &rules {
            let got = a.intersects_with(b);
            let both_one_per_row = a.rule() == GridRule::OnePerRow && b.rule() == GridRule::OnePerRow;
            if both_one_per_row {
                assert_eq!(None, got, "a={a:?}, b={b:?}");
                continue;
            }
            let want = verify_intersection(a, b);
            assert_eq!(Some(want), got, "a={a:?}, b={b:?}");
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let rule = grid(vec![vec![1, 2], vec![3, 4]], GridRule::OnePerRow);

    let json = serde_json::to_string(&rule).unwrap();
    assert_eq!(r#"{"grid":[[1,2],[3,4]],"rule":"OnePerRow"}"#, json);
    let decoded = serde_json::from_str::<GridQuorumSet<u64>>(&json).unwrap();
    assert_eq!(rule, decoded);

    let binary = bincode::serialize(&rule).unwrap();
    let decoded = bincode::deserialize::<GridQuorumSet<u64>>(&binary).unwrap();
    assert_eq!(rule, decoded);
}
