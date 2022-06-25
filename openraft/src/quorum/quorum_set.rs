/// A set of quorums is a collection of quorum.
///
/// A quorum is a collection of nodes that a read or write operation in distributed system has to contact to.
/// See: http://web.mit.edu/6.033/2005/wwwdocs/quorum_note.html
pub(crate) trait QuorumSet<ID: 'static> {
    /// Check if a series of ID constitute a quorum that is defined by this quorum set.
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool;
}
