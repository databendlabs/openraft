use std::collections::BTreeSet;

/// A set of quorums is a collection of quorum.
///
/// A quorum is a collection of nodes that a read or write operation in distributed system has to contact to.
/// See: http://web.mit.edu/6.033/2005/wwwdocs/quorum_note.html
pub(crate) trait QuorumSet<ID: 'static> {
    /// Check if a series of ID constitute a quorum that is defined by this quorum set.
    fn is_quorum<'a, I: Iterator<Item = &'a ID> + Clone>(&self, ids: I) -> bool;

    /// Returns all ids in this QuorumSet
    fn ids(&self) -> BTreeSet<ID>;
}

pub(crate) trait Coherent<ID, A, B>
where
    A: QuorumSet<ID>,
    B: QuorumSet<ID>,
{
    /// Returns if this QuorumSet is coherent with another.
    ///
    /// **Coherent** quorum set A and B is defined as: `∀ qᵢ ∈ A, qⱼ ∈ B: qᵢ ∩ qⱼ != ø`
    /// Openraft is only allowed to switch between two **coherent** quorum sets when changing membership.
    /// This is one of the two restrictions. The other restriction is to disable other smaller candidate to elect.
    fn is_coherent(a: &A, b: &B) -> bool;
}
