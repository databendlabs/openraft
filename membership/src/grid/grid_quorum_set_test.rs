use std::collections::BTreeSet;

use maplit::btreeset;
use quorum_set::verify_intersection;

use crate::errors::InvalidGrid;
use crate::quorum::ExplicitQuorumSet;
use crate::quorum::GridLine;
use crate::quorum::GridQuorumSet;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;
use crate::quorum::testing::accepted;
use crate::quorum::testing::all_subsets;
use crate::quorum::testing::arrangements;
use crate::quorum::testing::assert_quorum_set_contract;
use crate::quorum::testing::holding_one_of;
use crate::quorum::testing::small_grids;

/// Returns the rule over the valid `grid` that counts `line`.
fn grid(grid: Vec<Vec<u64>>, line: GridLine) -> GridQuorumSet<u64> {
    GridQuorumSet::new(grid, line).expect("the grid is valid")
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
        for line in [GridLine::Row, GridLine::Column] {
            let res = GridQuorumSet::new(cells.clone(), line);
            let want = Err(invalid.clone());
            assert_eq!(want, res, "grid={cells:?}, line={line:?}");
        }
    }
}

#[test]
fn test_rules_accept_the_sets_of_equal_explicit_lists() {
    // Each case lists a grid, its rows and its columns.
    let cases = [
        // A square grid.
        (
            vec![vec![1, 2], vec![3, 4]],
            vec![btreeset! {1, 2}, btreeset! {3, 4}],
            vec![btreeset! {1, 3}, btreeset! {2, 4}],
        ),
        // One row: the row rule needs every voter, and the column rule any one.
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
            vec![btreeset! {2, 4}, btreeset! {1, 5}, btreeset! {3, 6}],
        ),
    ];

    for (cells, rows, columns) in cases {
        let voters = cells.iter().flatten();
        let ids: BTreeSet<u64> = voters.copied().collect();
        let subsets = all_subsets(&ids);

        let row_rule = grid(cells.clone(), GridLine::Row);
        let explicit_rows = ExplicitQuorumSet::new(rows).unwrap();
        let got = accepted(&row_rule, &subsets);
        let want = accepted(&explicit_rows, &subsets);
        assert_eq!(want, got, "rows of {cells:?}");

        let column_rule = grid(cells.clone(), GridLine::Column);
        let explicit_columns = ExplicitQuorumSet::new(columns).unwrap();
        let got = accepted(&column_rule, &subsets);
        let want = accepted(&explicit_columns, &subsets);
        assert_eq!(want, got, "columns of {cells:?}");
    }
}

#[test]
fn test_small_rules_accept_exactly_the_sets_holding_a_complete_line() {
    for cells in small_grids() {
        let mut ids = BTreeSet::new();
        let mut rows = vec![];
        let mut columns = vec![BTreeSet::new(); cells[0].len()];
        for row in &cells {
            let row_ids: BTreeSet<u64> = row.iter().copied().collect();
            ids.extend(&row_ids);
            rows.push(row_ids);
            for (column, id) in row.iter().enumerate() {
                columns[column].insert(*id);
            }
        }
        let subsets = all_subsets(&ids);

        let row_rule = grid(cells.clone(), GridLine::Row);
        assert_quorum_set_contract(&row_rule);
        let got = accepted(&row_rule, &subsets);
        let want = holding_one_of(&rows, &subsets);
        assert_eq!(want, got, "rows of {cells:?}");

        let column_rule = grid(cells.clone(), GridLine::Column);
        assert_quorum_set_contract(&column_rule);
        let got = accepted(&column_rule, &subsets);
        let want = holding_one_of(&columns, &subsets);
        assert_eq!(want, got, "columns of {cells:?}");
    }
}

#[test]
fn test_rules_hold_their_grid() {
    let cells = vec![vec![4, 1, 6], vec![2, 5, 3]];
    let rows = GridQuorumSet::new(cells.clone(), GridLine::Row).unwrap();
    let columns = GridQuorumSet::new(cells.clone(), GridLine::Column).unwrap();

    let row_grid = rows.grid();
    assert_eq!(cells, row_grid);
    let column_grid = columns.grid();
    assert_eq!(cells, column_grid);
    let row_line = rows.line();
    assert_eq!(GridLine::Row, row_line);
    let column_line = columns.line();
    assert_eq!(GridLine::Column, column_line);

    let row_ids: Vec<u64> = rows.ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5, 6], row_ids);
    let column_ids: Vec<u64> = columns.ids().collect();
    assert_eq!(vec![1, 2, 3, 4, 5, 6], column_ids);
}

#[test]
fn test_is_quorum_ignores_unknown_and_repeated_ids() {
    let rows = grid(vec![vec![1, 2], vec![3, 4]], GridLine::Row);

    let accepts = rows.is_quorum([2, 9, 2, 1].iter());
    assert!(accepts);
    let accepts = rows.is_quorum([1, 1, 3, 9].iter());
    assert!(!accepts);
    let accepts = rows.is_quorum([].iter());
    assert!(!accepts);
}

#[test]
fn test_row_rule_and_column_rule_of_one_grid_intersect() {
    let grids = [
        vec![vec![1]],
        vec![vec![1, 2, 3]],
        vec![vec![1], vec![2], vec![3]],
        vec![vec![1, 2], vec![3, 4]],
        vec![vec![4, 1, 6], vec![2, 5, 3]],
    ];

    for cells in grids {
        let rows = grid(cells.clone(), GridLine::Row);
        let columns = grid(cells.clone(), GridLine::Column);

        let rows_meet_columns = rows.intersects_with(&columns);
        assert_eq!(Some(true), rows_meet_columns, "{cells:?}");
        let columns_meet_rows = columns.intersects_with(&rows);
        assert_eq!(Some(true), columns_meet_rows, "{cells:?}");

        // Two rows share no voter, and neither do two columns.
        let has_one_row = cells.len() == 1;
        let rows_meet_rows = rows.intersects_with(&rows);
        assert_eq!(Some(has_one_row), rows_meet_rows, "{cells:?}");
        let has_one_column = cells[0].len() == 1;
        let columns_meet_columns = columns.intersects_with(&columns);
        assert_eq!(Some(has_one_column), columns_meet_columns, "{cells:?}");
    }
}

#[test]
fn test_intersection_is_exact() {
    // Every grid of at most 4 cells, which holds distinct IDs of 1 to 4.
    let shapes = [(1, 1), (1, 2), (2, 1), (1, 3), (3, 1), (1, 4), (4, 1), (2, 2)];
    let mut rules = vec![];
    for (row_count, column_count) in shapes {
        for cells in arrangements(&[1, 2, 3, 4], row_count * column_count) {
            let rows = cells.chunks(column_count);
            let cells: Vec<Vec<u64>> = rows.map(|row| row.to_vec()).collect();
            rules.push(grid(cells.clone(), GridLine::Row));
            rules.push(grid(cells, GridLine::Column));
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
            let want = verify_intersection(a, b);
            let got = a.intersects_with(b);
            assert_eq!(Some(want), got, "a={a:?}, b={b:?}");
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn test_serde() {
    let rule = grid(vec![vec![1, 2], vec![3, 4]], GridLine::Column);

    let json = serde_json::to_string(&rule).unwrap();
    assert_eq!(r#"{"grid":[[1,2],[3,4]],"line":"Column"}"#, json);
    let decoded = serde_json::from_str::<GridQuorumSet<u64>>(&json).unwrap();
    assert_eq!(rule, decoded);

    let binary = bincode::serialize(&rule).unwrap();
    let decoded = bincode::deserialize::<GridQuorumSet<u64>>(&binary).unwrap();
    assert_eq!(rule, decoded);
}
