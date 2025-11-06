//! Progress tracks replication state, i.e., it can be considered a map of node id to already
//! replicated log id.
//!
//! The "progress" internally is a vector of scalar values.
//! The scalar value is monotonically incremental. Decreasing it is not allowed.
//! Optimization on calculating the quorum-accepted log id is done on this assumption.

#[cfg(feature = "bench")]
#[cfg(test)]
mod bench;
pub(crate) mod display_vec_progress;
pub(crate) mod entry;
pub(crate) mod id_val;
pub(crate) mod inflight;
pub(crate) mod progress_stats;
pub(crate) mod vec_progress;

use std::borrow::Borrow;
use std::slice::Iter;

use id_val::IdVal;
// TODO: remove it
#[allow(unused_imports)]
pub(crate) use inflight::Inflight;
pub(crate) use vec_progress::VecProgress;

use crate::quorum::QuorumSet;

/// Track the progress of several incremental values.
///
/// When one of the values is updated, it uses a `QuorumSet` to calculate the quorum-accepted
/// value. `ID` is the identifier of every progress value.
/// `V` is a type of progress entry.
/// `P` is the progress data of `V`, a progress entry `V` could contain other user data.
/// `QS` is a quorum set implementation.
pub(crate) trait Progress<ID, Ent, Prog, QS>
where
    ID: PartialEq + 'static,
    Ent: Borrow<Prog>,
    Prog: PartialOrd + Clone,
    QS: QuorumSet<ID>,
{
    /// Update one of the scalar values and re-calculate the quorum-accepted value with the provided
    /// function.
    ///
    /// It returns `Err(quorum_accepted)` if the `id` is not found.
    /// The provided function `f` updates the value of `id`.
    fn update_with<F>(&mut self, id: &ID, f: F) -> Result<&Prog, &Prog>
    where F: FnOnce(&mut Ent);

    /// Update one of the scalar values and re-calculate the quorum-accepted value.
    ///
    /// It returns `Err(quorum_accepted)` if the `id` is not found.
    fn update(&mut self, id: &ID, value: Ent) -> Result<&Prog, &Prog> {
        self.update_with(id, |x| *x = value)
    }

    /// Update the value if the new value is greater than the current value.
    ///
    /// It returns `Err(quorum_accepted)` if the `id` is not found.
    fn increase_to(&mut self, id: &ID, value: Ent) -> Result<&Prog, &Prog>
    where Ent: PartialOrd {
        self.update_with(id, |x| {
            if value > *x {
                *x = value;
            }
        })
    }

    /// Try to get the value by `id`.
    #[allow(dead_code)]
    fn try_get(&self, id: &ID) -> Option<&Ent>;

    /// Returns a mutable reference to the value corresponding to the `id`.
    fn get_mut(&mut self, id: &ID) -> Option<&mut Ent>;

    // TODO: merge `get` and `try_get`
    /// Get the value by `id`.
    #[allow(dead_code)]
    fn get(&self, id: &ID) -> &Ent;

    /// Get the greatest value that is accepted by a quorum defined in [`Self::quorum_set()`].
    ///
    /// In raft or other distributed consensus,
    /// To commit a value, the value has to be **accepted by a quorum** and has to be the greatest
    /// value every proposed.
    #[allow(dead_code)]
    fn quorum_accepted(&self) -> &Prog;

    /// Returns the reference to the quorum set
    #[allow(dead_code)]
    fn quorum_set(&self) -> &QS;

    /// Iterate over all id and values, voters first followed by learners.
    fn iter(&self) -> Iter<'_, IdVal<ID, Ent>>;

    /// Map each item to a value and collect into a collection.
    fn collect_mapped<F, T, C>(&self, f: F) -> C
    where
        F: Fn(&IdVal<ID, Ent>) -> T,
        C: FromIterator<T>,
    {
        self.iter().map(f).collect()
    }

    /// Build a new instance with the new quorum set, inheriting progress data from `self`.
    fn upgrade_quorum_set(
        self,
        quorum_set: QS,
        learner_ids: impl IntoIterator<Item = ID>,
        default_v: impl Fn() -> Ent,
    ) -> Self;

    /// Return if the given id is a voter.
    ///
    /// A voter is a node in the quorum set that can grant a value.
    /// A learner's progress is also tracked, but it will never grant a value.
    ///
    /// If the given id is not in this `Progress`, it returns `None`.
    fn is_voter(&self, id: &ID) -> Option<bool>;
}
