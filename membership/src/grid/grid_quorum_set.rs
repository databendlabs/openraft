use std::collections::BTreeSet;
use std::collections::btree_set;

use openraft::NodeId;
use openraft_macros::since;

use crate::errors::InvalidGrid;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// The lines of a grid that a [`GridQuorumSet`] counts: its rows or its columns.
#[since(version = "0.10.0")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum GridLine {
    /// An accepted set holds every voter of some row.
    Row,

    /// An accepted set holds every voter of some column.
    Column,
}

/// A quorum rule that accepts an ID set holding every voter of some row of its grid, or of some
/// column.
///
/// The grid is a rectangle of distinct voters, given as its rows. [`GridLine`] selects whether an
/// accepted set holds a complete row or a complete column.
///
/// The rows of a grid are disjoint, so no row holds another, and the minimal quorums of the row
/// rule are exactly the rows. Likewise, the minimal quorums of the column rule are exactly the
/// columns. Every row shares one cell with every column, so the row rule and the column rule of
/// one grid always intersect.
///
/// # Examples
///
/// ```
/// use openraft::membership::{QuorumIntersection, QuorumSet};
/// use openraft_membership::quorum::{GridLine, GridQuorumSet};
///
/// let grid = vec![vec![1, 2, 3], vec![4, 5, 6]];
/// let rows = GridQuorumSet::new(grid.clone(), GridLine::Row).unwrap();
/// let columns = GridQuorumSet::new(grid, GridLine::Column).unwrap();
///
/// let accepts = rows.is_quorum([4, 5, 6].iter());
/// assert!(accepts);
/// let accepts = columns.is_quorum([2, 5].iter());
/// assert!(accepts);
/// let accepts = columns.is_quorum([1, 2, 3].iter());
/// assert!(!accepts);
///
/// // Every row meets every column, but row {1,2,3} misses row {4,5,6}.
/// let intersects = rows.intersects_with(&columns);
/// assert_eq!(Some(true), intersects);
/// let intersects = rows.intersects_with(&rows);
/// assert_eq!(Some(false), intersects);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound(deserialize = "ID: NodeId"))
)]
pub struct GridQuorumSet<ID> {
    /// The rows of the grid, each one the voters of its cells.
    grid: Vec<Vec<ID>>,

    /// The lines whose voters an accepted set holds.
    line: GridLine,
}

impl<ID> GridQuorumSet<ID>
where ID: NodeId
{
    /// Builds a rule over `grid`, given as its rows, that accepts an ID set holding every voter
    /// of some `line`.
    ///
    /// Returns [`InvalidGrid`] unless `grid` has a row and a column, every row is as long as the
    /// first, and no voter is in two cells.
    pub fn new(grid: Vec<Vec<ID>>, line: GridLine) -> Result<Self, InvalidGrid<ID>> {
        check_grid(&grid)?;
        let rule = Self { grid, line };
        Ok(rule)
    }

    /// Returns the grid, as its rows.
    pub fn grid(&self) -> &[Vec<ID>] {
        &self.grid
    }

    /// Returns the lines whose voters an accepted set holds.
    pub fn line(&self) -> GridLine {
        self.line
    }

    /// Returns the voters of each `line` of `grid`: of each row, or of each column.
    fn lines(grid: &[Vec<ID>], line: GridLine) -> Vec<BTreeSet<&ID>> {
        match line {
            GridLine::Row => {
                let mut rows = Vec::new();
                for row in grid.iter() {
                    let cells = row.iter().collect();
                    rows.push(cells);
                }
                rows
            }
            GridLine::Column => {
                // Safe index: the grid has a row, and every row has a cell in each column.
                let column_count = grid[0].len();
                let mut columns = Vec::new();
                for column in 0..column_count {
                    let cells = grid.iter().map(|row| &row[column]);
                    let cells = cells.collect();
                    columns.push(cells);
                }
                columns
            }
        }
    }
}

impl<ID> QuorumSet for GridQuorumSet<ID>
where ID: NodeId
{
    type Id = ID;
    type Iter = btree_set::IntoIter<ID>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where
        ID: 'a,
        I: Iterator<Item = &'a ID> + Clone,
    {
        // This checks each line in place instead of building it with `lines()`, because
        // replication calls `is_quorum()` on every progress update.
        let candidates: BTreeSet<&ID> = ids.collect();
        match self.line {
            GridLine::Row => {
                for row in self.grid.iter() {
                    let holds_row = row.iter().all(|id| candidates.contains(id));
                    if holds_row {
                        return true;
                    }
                }
            }
            GridLine::Column => {
                // Safe index: the grid has a row, and every row has a cell in each column.
                let column_count = self.grid[0].len();
                for column in 0..column_count {
                    let holds_column = self.grid.iter().all(|row| candidates.contains(&row[column]));
                    if holds_column {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn ids(&self) -> Self::Iter {
        let cells = self.grid.iter().flatten();
        let ids: BTreeSet<ID> = cells.cloned().collect();
        ids.into_iter()
    }
}

/// Exact quorum intersection for two grid rules.
///
/// Every quorum of a rule holds one of its lines, and every line is a quorum. Two quorums are
/// therefore disjoint only when two lines are, so the check is exact. It compares each line of one
/// rule with each line of the other.
impl<ID> QuorumIntersection<GridQuorumSet<ID>> for GridQuorumSet<ID>
where ID: NodeId
{
    /// Return `Some(false)` when a line of `self` and a line of `other` share no voter, and
    /// `Some(true)` otherwise. This check never returns `None`.
    fn intersects_with(&self, other: &GridQuorumSet<ID>) -> Option<bool> {
        let lines = Self::lines(&self.grid, self.line);
        let other_lines = Self::lines(&other.grid, other.line);
        for line in &lines {
            for other_line in &other_lines {
                if line.is_disjoint(other_line) {
                    return Some(false);
                }
            }
        }
        Some(true)
    }
}

/// Returns an error unless `grid` has a row and a column, every row is as long as the first, and
/// no voter is in two cells.
fn check_grid<ID>(grid: &[Vec<ID>]) -> Result<(), InvalidGrid<ID>>
where ID: NodeId {
    let Some(first_row) = grid.first() else {
        return Err(InvalidGrid::NoRow);
    };
    let column_count = first_row.len();
    if column_count == 0 {
        return Err(InvalidGrid::NoColumn);
    }

    for (row, cells) in grid.iter().enumerate() {
        let len = cells.len();
        if len != column_count {
            let ragged = InvalidGrid::RaggedRow { row, len, column_count };
            return Err(ragged);
        }
    }

    let mut voters = BTreeSet::new();
    for voter in grid.iter().flatten() {
        let is_new = voters.insert(voter);
        if !is_new {
            let duplicate = InvalidGrid::DuplicateVoter { voter: voter.clone() };
            return Err(duplicate);
        }
    }
    Ok(())
}
