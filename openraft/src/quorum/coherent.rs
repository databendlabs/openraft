use crate::quorum::QuorumSet;

/// **Coherent** quorum set A and B is defined as: `∀ qᵢ ∈ A, ∀ qⱼ ∈ B: qᵢ ∩ qⱼ != ø`, i.e., `A ~
/// B`.
/// A distributed consensus protocol such as openraft is only allowed to switch membership
/// between two **coherent** quorum sets. Being coherent is one of the two restrictions. The other
/// restriction is to disable another smaller candidate to elect.
pub(crate) trait Coherent<ID, Other>
where
    ID: PartialOrd + Ord + 'static,
    Self: QuorumSet<ID>,
    Other: QuorumSet<ID>,
{
    /// Returns `true` if this QuorumSet is coherent with the other quorum set.
    fn is_coherent_with(&self, other: &Other) -> bool;
}

pub(crate) trait FindCoherent<ID, Other>
where
    ID: PartialOrd + Ord + 'static,
    Self: QuorumSet<ID>,
    Other: QuorumSet<ID>,
{
    /// Build a QuorumSet `X` so that `self` is coherent with `X` and `X` is coherent with `other`,
    /// i.e., `self ~ X ~ other`.
    /// Then `X` is the intermediate QuorumSet when changing membership from `self` to `other`.
    ///
    /// E.g.(`cᵢcⱼ` is a joint of `cᵢ` and `cⱼ`):
    /// - `c₁.find_coherent(c₁)`   returns `c₁`
    /// - `c₁.find_coherent(c₂)`   returns `c₁c₂`
    /// - `c₁c₂.find_coherent(c₂)` returns `c₂`
    /// - `c₁c₂.find_coherent(c₁)` returns `c₁`
    /// - `c₁c2.find_coherent(c₃)` returns `c₂c₃`
    fn find_coherent(&self, other: Other) -> Self;
}
