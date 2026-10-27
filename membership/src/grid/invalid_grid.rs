use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft_macros::since;

/// The grid of a [`GridMembership`] config is not a rectangle of distinct voters with at least one
/// row and one column.
///
/// [`GridQuorumSet::new()`] returns it, and [`GridMembership`] and its edits return it in
/// [`MembershipError::UserDefined`]. OpenRaft wraps it with the configured error source when
/// applying an edit inside the Raft core.
///
/// [`GridMembership`]: crate::GridMembership
/// [`GridQuorumSet::new()`]: crate::quorum::GridQuorumSet::new
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidGrid<NID>
where NID: NodeId
{
    /// The grid has no row, so it has no voter.
    #[error("the grid has no row")]
    NoRow,

    /// Row 0 is empty, so the grid has no column.
    #[error("the grid has no column")]
    NoColumn,

    /// A row is shorter or longer than row 0, so the grid is not a rectangle.
    #[error("row {row} has length {len}, but row 0 has length {column_count}")]
    RaggedRow {
        /// The index of the first row whose length differs from the length of row 0.
        row: usize,

        /// The length of that row.
        len: usize,

        /// The length of row 0, the number of columns.
        column_count: usize,
    },

    /// A voter is in more than one cell. Each cell holds its own voter.
    #[error("voter {voter} is in more than one cell")]
    DuplicateVoter {
        /// The voter of the first cell, in row-major order, that repeats an earlier cell.
        voter: NID,
    },
}

impl<NID> From<InvalidGrid<NID>> for MembershipError<NID, InvalidGrid<NID>>
where NID: NodeId
{
    fn from(e: InvalidGrid<NID>) -> Self {
        MembershipError::UserDefined(e)
    }
}
