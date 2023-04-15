use crate::compat::Compat;
use crate::compat::Upgrade;

/// v0.7 compatible Vote(in v0.7 the corresponding type is `HardState`).
///
/// To load from either v0.7 or the latest format data and upgrade it to latest type:
/// ```ignore
/// let x: openraft::Vote = serde_json::from_slice::<compat07::Vote>(&serialized_bytes)?.upgrade()
/// ```
pub type Vote = Compat<or07::HardState, crate::Vote<u64>>;

impl Upgrade<crate::Vote<u64>> for or07::HardState {
    fn upgrade(self) -> crate::Vote<u64> {
        // When it has not yet voted for any node, let it vote for any node won't break the consensus.
        crate::Vote::new(self.current_term, self.voted_for.unwrap_or_default())
    }
}
