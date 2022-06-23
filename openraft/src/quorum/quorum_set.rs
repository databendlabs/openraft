/// A set of quorums
pub(crate) trait QuorumSet<ID: 'static> {
    /// Check if a series of ID constitute a quorum that is defined by this quorum set.
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool;
}
