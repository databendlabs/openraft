use std::sync::Arc;

/// A set of quorums is a collection of quorum.
///
/// A quorum is a collection of nodes that a read or write operation in a distributed system has to
/// contact. See: <http://web.mit.edu/6.033/2005/wwwdocs/quorum_note.html>
///
/// Implementations must be upward-closed: adding more IDs to a quorum must still be a quorum.
pub(crate) trait QuorumSet {
    type Id: 'static;

    type Iter: Iterator<Item = Self::Id>;

    /// Check if a series of ID constitute a quorum that is defined by this quorum set.
    fn is_quorum<'a, I: Iterator<Item = &'a Self::Id> + Clone>(&self, ids: I) -> bool;

    /// Returns all ids in this QuorumSet
    fn ids(&self) -> Self::Iter;
}

impl<T: QuorumSet> QuorumSet for Arc<T> {
    type Id = T::Id;

    type Iter = T::Iter;

    fn is_quorum<'a, I: Iterator<Item = &'a Self::Id> + Clone>(&self, ids: I) -> bool {
        self.as_ref().is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.as_ref().ids()
    }
}
