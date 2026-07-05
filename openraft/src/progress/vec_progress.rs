use std::fmt::Display;
use std::fmt::Formatter;
use std::slice::Iter;
use std::slice::IterMut;

use super::VecProgressEntry;
use super::VecProgressEntryData;
use super::display_vec_progress::DisplayVecProgress;
use super::progress_stats::ProgressStats;
use crate::quorum::QuorumSet;

/// Track the progress of several incremental values.
///
/// When one of the values is updated, it uses a `QuorumSet` to calculate the quorum-accepted
/// value.
/// `Entry` stores an ID, a progress value, and application-owned user data.
/// `QS` is a quorum set implementation.
///
/// Internally it uses a vector as storage, thus it is suitable for a small quorum set.
#[derive(Clone, Debug)]
pub(crate) struct VecProgress<Entry, QS>
where
    Entry: VecProgressEntry,
    QS: QuorumSet<Id = Entry::Id>,
{
    /// Quorum set to determine if a set of `id` constitutes a quorum.
    quorum_set: QS,

    /// The max value that is accepted by a quorum.
    quorum_accepted: Entry::Progress,

    /// Number of voters
    voter_count: usize,

    /// Progress data.
    ///
    /// Elements with values greater than the `quorum_accepted` are sorted in descending order.
    /// Others are unsorted.
    ///
    /// The first `voter_count` elements are voters, the left are learners.
    /// Learner elements are always still.
    /// A voter element will be moved up to keep them in descending order when a new value is
    /// updated.
    entries: Vec<Entry>,

    /// Statistics of how it runs.
    stat: ProgressStats,
}

impl<Entry, QS> Display for VecProgress<Entry, QS>
where
    Entry: VecProgressEntry + Display,
    QS: QuorumSet<Id = Entry::Id> + 'static,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, item) in self.entries.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", item)?
        }
        write!(f, "}}")?;

        Ok(())
    }
}

impl<Entry, QS> VecProgress<Entry, QS>
where
    Entry: VecProgressEntry,
    QS: QuorumSet<Id = Entry::Id>,
{
    /// Create a progress tracker from quorum and learner IDs.
    pub(crate) fn new(
        quorum_set: QS,
        learner_ids: impl IntoIterator<Item = Entry::Id>,
        mut default_entry: impl FnMut(Entry::Id) -> Entry,
    ) -> Self {
        let mut entries = quorum_set.ids().map(&mut default_entry).collect::<Vec<_>>();

        let voter_count = entries.len();

        entries.extend(learner_ids.into_iter().map(default_entry));

        Self {
            quorum_set,
            quorum_accepted: Default::default(),
            voter_count,
            entries,
            stat: Default::default(),
        }
    }

    /// Find the index of the specified id.
    #[inline(always)]
    pub(crate) fn index(&self, target: &Entry::Id) -> Option<usize> {
        self.entries.iter().position(|item| item.id() == target)
    }

    /// Move an element at `index` up so that all the values greater than `quorum_accepted` are
    /// sorted.
    #[inline(always)]
    fn move_up(&mut self, index: usize) -> usize {
        self.stat.move_count += 1;
        for i in (0..index).rev() {
            if self.entries[i].progress() < self.entries[i + 1].progress() {
                self.entries.swap(i, i + 1);
            } else {
                return i + 1;
            }
        }

        0
    }

    /// Move a voter element at `index` down so that all the values greater than `quorum_accepted`
    /// stay sorted after its progress value is lowered.
    ///
    /// It is the counterpart of [`Self::move_up`], used by [`Self::reset_entry_with()`].
    fn move_down(&mut self, index: usize) -> usize {
        self.stat.move_count += 1;
        let mut i = index;
        while i + 1 < self.voter_count && self.entries[i].progress() < self.entries[i + 1].progress() {
            self.entries.swap(i, i + 1);
            i += 1;
        }

        i
    }

    /// Return mutable entries without maintaining the progress ordering.
    ///
    /// Mutating progress values through this iterator can leave the internal
    /// ordering and quorum-accepted value stale. Normal progress updates must
    /// use `update()`, `update_with()`, or `update_entry_with()` instead.
    /// Mutating entry IDs can corrupt membership lookup.
    pub(crate) fn iter_mut_without_reorder(&mut self) -> IterMut<'_, Entry> {
        self.entries.iter_mut()
    }

    pub(crate) fn into_iter(self) -> impl Iterator<Item = Entry> {
        self.entries.into_iter()
    }

    #[cfg(test)]
    pub(crate) fn stat(&self) -> &ProgressStats {
        &self.stat
    }

    pub(crate) fn display_with<Fmt>(&self, f: Fmt) -> DisplayVecProgress<'_, Entry, QS, Fmt>
    where Fmt: Fn(&mut Formatter<'_>, &Entry) -> std::fmt::Result {
        DisplayVecProgress { inner: self, f }
    }
}

impl<Entry, QS> VecProgress<Entry, QS>
where
    Entry: VecProgressEntry,
    QS: QuorumSet<Id = Entry::Id>,
{
    /// Update one of the scalar values and re-calculate the quorum-accepted value.
    ///
    /// It returns `Err(quorum_accepted)` if the `id` is not found.
    /// Re-updating with the same V will do nothing.
    ///
    /// # Algorithm
    ///
    /// Only when the **previous value** is less than or equal to the quorum-accepted,
    /// and the **new value** is greater than the quorum-accepted
    /// there is possibly an update to the quorum-accepted.
    ///
    /// This way it gets rid of a portion of unnecessary re-calculation of quorum-accepted
    /// and avoids unnecessary sorting: progresses are kept in order, and only values greater than
    /// quorum-accepted need to sort.
    ///
    /// E.g., given 3 ids with values `1,3,5`, as shown in the figure below:
    ///
    /// ```text
    /// a -----------+-------->
    /// b -------+------------>
    /// c ---+---------------->
    /// ------------------------------
    ///      1   3   5
    /// ```
    ///
    /// the quorum-accepted is `3` and assumes a majority quorum set is used.
    /// Then:
    /// - update(a, 6): nothing to do: quorum-accepted is still 3;
    /// - update(b, 4): re-calc:       quorum-accepted becomes 4;
    /// - update(b, 6): re-calc:       quorum-accepted becomes 5;
    /// - update(c, 2): nothing to do: quorum-accepted is still 3;
    /// - update(c, 3): nothing to do: quorum-accepted is still 3;
    /// - update(c, 4): re-calc:       quorum-accepted becomes 4;
    /// - update(c, 6): re-calc:       quorum-accepted becomes 5;
    pub(crate) fn update_with<F>(&mut self, id: &Entry::Id, f: F) -> Result<&Entry::Progress, &Entry::Progress>
    where F: FnOnce(&mut Entry::Progress) {
        self.update_entry_with(id, |entry| f(entry.progress_mut()))
    }

    /// Update an entry and re-calculate the quorum-accepted value.
    ///
    /// This is for application-owned fields that have to be mutated together
    /// with the progress value. The progress update must still be monotonic.
    /// The entry ID must not be changed.
    pub(crate) fn update_entry_with<F>(&mut self, id: &Entry::Id, f: F) -> Result<&Entry::Progress, &Entry::Progress>
    where F: FnOnce(&mut Entry) {
        self.stat.update_count += 1;

        let index = match self.index(id) {
            None => {
                return Err(&self.quorum_accepted);
            }
            Some(x) => x,
        };

        let prev_progress = self.entries[index].progress().clone();

        f(&mut self.entries[index]);

        debug_assert!(self.entries[index].id() == id);

        self.update_at(index, prev_progress)
    }

    /// Update application-owned data without recalculating quorum-accepted progress.
    ///
    /// This method only exposes [`VecProgressEntryData::Data`], so it cannot
    /// change progress or invalidate the ordering maintained by `VecProgress`.
    ///
    /// It returns the updated data after update if the `id` is found, otherwise returns `None`.
    pub(crate) fn update_data_with<F>(&mut self, id: &Entry::Id, f: F) -> Option<&Entry::Data>
    where
        Entry: VecProgressEntryData,
        F: FnOnce(&mut Entry::Data),
    {
        let index = self.index(id)?;

        f(self.entries[index].data_mut());

        Some(self.entries[index].data())
    }

    /// Update an entry whose progress value may move backward, e.g., when replication progress
    /// is reset upon log reversion.
    ///
    /// If the progress value is lowered, the entry is moved down to keep the values greater than
    /// `quorum_accepted` sorted. The recorded quorum-accepted value is deliberately not
    /// recalculated: a value accepted by a quorum must never be withdrawn.
    /// The entry ID must not be changed.
    ///
    /// It returns the updated entry if the `id` is found, otherwise returns `None`.
    pub(crate) fn reset_entry_with<F>(&mut self, id: &Entry::Id, f: F) -> Option<&Entry>
    where F: FnOnce(&mut Entry) {
        let index = self.index(id)?;

        let prev_progress = self.entries[index].progress().clone();

        f(&mut self.entries[index]);

        debug_assert!(self.entries[index].id() == id);
        debug_assert!(self.entries[index].progress() <= &prev_progress);

        // Learners are never reordered.
        let new_index = if index < self.voter_count && self.entries[index].progress() < &prev_progress {
            self.move_down(index)
        } else {
            index
        };

        Some(&self.entries[new_index])
    }

    fn update_at(
        &mut self,
        index: usize,
        prev_progress: Entry::Progress,
    ) -> Result<&Entry::Progress, &Entry::Progress> {
        debug_assert!(self.entries[index].progress() >= &prev_progress,);

        // No change, return early
        if &prev_progress == self.entries[index].progress() {
            return Ok(&self.quorum_accepted);
        }

        // Learner does not grant a value.
        // And it won't be moved up to adjust the order.
        if index >= self.voter_count {
            return Ok(&self.quorum_accepted);
        }

        let prev_le_qa = prev_progress <= self.quorum_accepted;
        let new_gt_qa = self.entries[index].progress() > &self.quorum_accepted;

        // Sort and find the greatest value accepted by a quorum set.

        if new_gt_qa {
            let new_index = self.move_up(index);

            if prev_le_qa {
                // From high to low, find the max value that has constituted a quorum.
                for i in new_index..self.voter_count {
                    let prog = self.entries[i].progress();

                    // No need to re-calculate already quorum-accepted value.
                    if prog <= &self.quorum_accepted {
                        break;
                    }

                    // Ids of the target that has value GE `entries[i]`
                    let it = self.entries[0..=i].iter().map(|item| item.id());

                    self.stat.is_quorum_count += 1;

                    if self.quorum_set.is_quorum(it) {
                        self.quorum_accepted = prog.clone();
                        break;
                    }
                }
            }
        }

        Ok(&self.quorum_accepted)
    }

    /// Update one of the scalar values and re-calculate the quorum-accepted value.
    ///
    /// It returns `Err(quorum_accepted)` if the `id` is not found.
    pub(crate) fn update(
        &mut self,
        id: &Entry::Id,
        value: Entry::Progress,
    ) -> Result<&Entry::Progress, &Entry::Progress> {
        self.update_with(id, |x| *x = value)
    }

    /// Update the value if the new value is greater than the current value.
    ///
    /// It returns `Err(quorum_accepted)` if the `id` is not found.
    pub(crate) fn increase_to(
        &mut self,
        id: &Entry::Id,
        value: Entry::Progress,
    ) -> Result<&Entry::Progress, &Entry::Progress> {
        self.update_with(id, |x| {
            if value > *x {
                *x = value;
            }
        })
    }

    /// Try to get the value by `id`.
    pub(crate) fn try_get(&self, id: &Entry::Id) -> Option<&Entry> {
        let index = self.index(id)?;
        Some(&self.entries[index])
    }

    // TODO: merge `get` and `try_get`
    /// Get the value by `id`.
    #[cfg(test)]
    pub(crate) fn get(&self, id: &Entry::Id) -> &Entry {
        let index = self.index(id).unwrap();
        &self.entries[index]
    }

    /// Get the greatest value that is accepted by the quorum set.
    ///
    /// In raft or other distributed consensus,
    /// To commit a value, the value has to be **accepted by a quorum** and has to be the greatest
    /// value every proposed.
    #[cfg(test)]
    pub(crate) fn quorum_accepted(&self) -> &Entry::Progress {
        &self.quorum_accepted
    }

    /// Iterate over all id and values, voters first followed by learners.
    pub(crate) fn iter(&self) -> Iter<'_, Entry> {
        self.entries.as_slice().iter()
    }

    /// Map each item to a value and collect into a collection.
    pub(crate) fn collect_mapped<F, T, C>(&self, f: F) -> C
    where
        F: Fn(&Entry) -> T,
        C: FromIterator<T>,
    {
        self.iter().map(f).collect()
    }

    /// Build a new instance with the new quorum set, inheriting progress data from `self`.
    pub(crate) fn upgrade_quorum_set(
        self,
        quorum_set: QS,
        learner_ids: impl IntoIterator<Item = Entry::Id>,
        default_entry: impl FnMut(Entry::Id) -> Entry,
    ) -> Self {
        let mut new_prog = Self::new(quorum_set, learner_ids, default_entry);

        new_prog.stat = self.stat.clone();

        for item in self.into_iter() {
            new_prog.replace(item).ok();
        }
        new_prog
    }

    /// Replace the entry for the same ID and update quorum-accepted progress.
    fn replace(&mut self, entry: Entry) -> Result<&Entry::Progress, &Entry::Progress> {
        self.stat.update_count += 1;

        let index = match self.index(entry.id()) {
            None => {
                return Err(&self.quorum_accepted);
            }
            Some(x) => x,
        };

        let prev_progress = self.entries[index].progress().clone();

        self.entries[index] = entry;

        self.update_at(index, prev_progress)
    }

    /// Return if the given id is a voter.
    ///
    /// A voter is a node in the quorum set that can grant a value.
    /// A learner's progress is also tracked, but it will never grant a value.
    ///
    /// If the given id is not in this `VecProgress`, it returns `None`.
    pub(crate) fn is_voter(&self, id: &Entry::Id) -> Option<bool> {
        let index = self.index(id)?;
        Some(index < self.voter_count)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    use maplit::btreeset;

    use super::VecProgress;
    use crate::progress::id_val::IdValData;
    use crate::quorum::QuorumSet;

    const LCG_A: u64 = 6364136223846793005;
    const LCG_C: u64 = 1442695040888963407;

    #[derive(Clone, Debug)]
    struct RequiredSetQuorum {
        ids: BTreeSet<u64>,
        required: BTreeSet<u64>,
    }

    impl RequiredSetQuorum {
        /// Build a quorum set that grants only when every required ID is present.
        ///
        /// This intentionally does not implement majority semantics, so it can
        /// catch accidental assumptions in `VecProgress`.
        fn new(ids: impl IntoIterator<Item = u64>, required: impl IntoIterator<Item = u64>) -> Self {
            Self {
                ids: ids.into_iter().collect(),
                required: required.into_iter().collect(),
            }
        }
    }

    impl QuorumSet for RequiredSetQuorum {
        type Id = u64;

        type Iter = std::collections::btree_set::IntoIter<u64>;

        fn is_quorum<'a, I: Iterator<Item = &'a Self::Id> + Clone>(&self, ids: I) -> bool {
            let granted = ids.copied().collect::<BTreeSet<_>>();
            self.required.is_subset(&granted)
        }

        fn ids(&self) -> Self::Iter {
            self.ids.clone().into_iter()
        }
    }

    /// Advance a deterministic pseudo-random sequence for model-based tests.
    ///
    /// The generator is intentionally tiny and reproducible; it is not used for
    /// randomness quality, only for broadening the monotonic update cases.
    fn next_random(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_mul(LCG_A).wrapping_add(LCG_C);
        *seed
    }

    /// Build learner IDs as the complement of the current quorum IDs.
    ///
    /// Randomized upgrade tests use this to preserve progress for all known
    /// nodes when switching between simple, joint, and shrunk quorum sets.
    fn learner_ids_for<QS>(quorum_set: &QS, known_ids: impl IntoIterator<Item = u64>) -> Vec<u64>
    where QS: QuorumSet<Id = u64> {
        let voter_ids = quorum_set.ids().collect::<BTreeSet<_>>();
        known_ids.into_iter().filter(|id| !voter_ids.contains(id)).collect()
    }

    /// Copy both sides of a `Result<&u64, &u64>` returned by progress updates.
    ///
    /// `Result::copied()` only copies the `Ok` value, but these tests compare
    /// both successful updates and missing-id errors by value.
    fn copy_result(res: Result<&u64, &u64>) -> Result<u64, u64> {
        match res {
            Ok(x) => Ok(*x),
            Err(x) => Err(*x),
        }
    }

    /// Compute quorum-accepted progress with a straightforward reference model.
    ///
    /// This intentionally ignores `VecProgress`'s internal ordering optimization:
    /// it tries candidate progress values from high to low and returns the first
    /// value whose reached node IDs form a quorum.
    fn model_quorum_accepted<QS>(quorum_set: &QS, entries: &[(u64, u64)]) -> u64
    where QS: QuorumSet<Id = u64> {
        let values = entries.iter().map(|item| (item.0, item.1)).collect::<BTreeMap<_, _>>();
        let mut candidates = quorum_set.ids().map(|id| values[&id]).collect::<Vec<_>>();

        candidates.sort_unstable_by(|a, b| b.cmp(a));
        candidates.dedup();

        for candidate in candidates {
            let ids = values.iter().filter_map(|(id, val)| (*val >= candidate).then_some(id));
            if quorum_set.is_quorum(ids) {
                return candidate;
            }
        }

        0
    }

    /// Assert that `VecProgress` agrees with the reference model.
    ///
    /// This checks both the externally visible quorum-accepted value and the
    /// internal ordering invariant relied on by the optimized update algorithm.
    fn assert_matches_model<QS>(progress: &VecProgress<(u64, u64), QS>, context: &str)
    where QS: QuorumSet<Id = u64> {
        let want = model_quorum_accepted(&progress.quorum_set, &progress.entries);
        assert_eq!(
            &want,
            progress.quorum_accepted(),
            "{}: entries: {:?}",
            context,
            progress.entries
        );
        assert_voter_prefix_is_sorted(progress, context);
    }

    /// Assert the voter ordering invariant maintained by `update()`.
    ///
    /// Voter entries above `quorum_accepted` must form a descending prefix.
    /// Learners are excluded because learner progress does not grant quorum.
    fn assert_voter_prefix_is_sorted<QS>(progress: &VecProgress<(u64, u64), QS>, context: &str)
    where QS: QuorumSet<Id = u64> {
        let quorum_accepted = *progress.quorum_accepted();
        let mut previous = None;
        let mut seen_not_above_quorum = false;

        for item in &progress.entries[..progress.voter_count] {
            if item.1 <= quorum_accepted {
                seen_not_above_quorum = true;
                continue;
            }

            assert!(
                !seen_not_above_quorum,
                "{}: non-prefix above-quorum entry: {:?}",
                context, progress.entries
            );
            if let Some(prev) = previous {
                assert!(prev >= item.1, "{}: unsorted voters: {:?}", context, progress.entries);
            }
            previous = Some(item.1);
        }
    }

    #[test]
    fn vec_progress_new() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6, 7], |id| (id, 0));

        assert_eq!(
            vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (6, 0), (7, 0),],
            progress.entries
        );
        assert_eq!(5, progress.voter_count);
    }

    #[test]
    fn vec_progress_tuple_entry() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [3], |id| (id, 0));

        assert_eq!(
            vec![(0, 0), (1, 0), (2, 0), (3, 0)],
            progress.iter().cloned().collect::<Vec<_>>()
        );
        assert_eq!(Ok(&0), progress.update(&0, 5));
        assert_eq!(Ok(&5), progress.update(&1, 5));
        assert_eq!(Some(&(0, 5)), progress.try_get(&0));
        assert_eq!(Some(&(1, 5)), progress.try_get(&1));
        assert_eq!(&5, progress.quorum_accepted());
    }

    #[test]
    fn vec_progress_index() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6, 7], |id| (id, 0));

        assert_eq!(Some(0), progress.index(&0));
        assert_eq!(Some(1), progress.index(&1));
        assert_eq!(Some(4), progress.index(&4));
        assert_eq!(Some(5), progress.index(&6));
        assert_eq!(Some(6), progress.index(&7));
        assert_eq!(None, progress.index(&9));
        assert_eq!(None, progress.index(&100));
    }

    #[test]
    fn vec_progress_get() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6, 7], |id| (id, 0));

        progress.update(&6, 5).ok();
        assert_eq!(&5, &progress.get(&6).1);
        assert_eq!(Some(&5), progress.try_get(&6).map(|x| &x.1));
        assert_eq!(None, progress.try_get(&9));

        progress.update(&6, 10).ok();
        assert_eq!(Some(&10), progress.try_get(&6).map(|x| &x.1));
    }

    #[test]
    fn vec_progress_iter() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6, 7], |id| (id, 0));

        progress.update(&7, 7).ok();
        progress.update(&3, 3).ok();
        progress.update(&1, 1).ok();

        assert_eq!(
            vec![(3, 3), (1, 1), (0, 0), (2, 0), (4, 0), (6, 0), (7, 7),],
            progress.iter().cloned().collect::<Vec<_>>(),
            "iter() returns voter first, followed by learners"
        );
    }

    #[test]
    fn vec_progress_move_up() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6], |id| (id, 0));

        // initial: 0-0, 1-0, 2-0, 3-0, 4-0
        let cases = [
            ((1, 2), vec![(1, 2), (0, 0), (2, 0), (3, 0), (4, 0), (6, 0)], 0),
            ((2, 3), vec![(2, 3), (1, 2), (0, 0), (3, 0), (4, 0), (6, 0)], 0),
            ((1, 3), vec![(2, 3), (1, 3), (0, 0), (3, 0), (4, 0), (6, 0)], 1), // no move
            ((4, 8), vec![(4, 8), (2, 3), (1, 3), (0, 0), (3, 0), (6, 0)], 0),
            ((0, 5), vec![(4, 8), (0, 5), (2, 3), (1, 3), (3, 0), (6, 0)], 1), // move to 1st
        ];
        for (ith, ((id, v), want_vec, want_new_index)) in cases.iter().enumerate() {
            // Update a value and move it up to keep the order.
            let index = progress.index(id).unwrap();
            progress.entries[index].1 = *v;
            let got = progress.move_up(index);

            assert_eq!(want_vec, &progress.entries, "{}-th case: idx:{}, v:{}", ith, *id, *v);
            assert_eq!(*want_new_index, got, "{}-th case: idx:{}, v:{}", ith, *id, *v);
        }
    }

    #[test]
    fn vec_progress_update() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6], |id| (id, 0));

        // initial: 0,0,0,0,0
        let cases = vec![
            ((6, 9), Ok(&0)),  // 0,0,0,0,0,9 // learner won't affect quorum-accepted
            ((1, 2), Ok(&0)),  // 0,2,0,0,0,0
            ((2, 3), Ok(&0)),  // 0,2,3,0,0,0
            ((3, 1), Ok(&1)),  // 0,2,3,1,0,0
            ((4, 5), Ok(&2)),  // 0,2,3,1,5,0
            ((0, 4), Ok(&3)),  // 4,2,3,1,5,0
            ((3, 2), Ok(&3)),  // 4,2,3,2,5,0
            ((3, 3), Ok(&3)),  // 4,2,3,2,5,0
            ((1, 4), Ok(&4)),  // 4,4,3,2,5,0
            ((9, 1), Err(&4)), // nonexistent id, ignore.
        ];

        for (ith, ((id, v), want_quorum_accepted)) in cases.iter().enumerate() {
            let got = progress.update_with(id, |x| *x = *v);
            assert_eq!(want_quorum_accepted.clone(), got, "{}-th case: id:{}, v:{}", ith, id, v);
        }
    }

    #[test]
    fn vec_progress_matches_reference_model_for_monotonic_updates() {
        let cases = [
            (vec![btreeset! {0, 1, 2, 3, 4}], vec![5, 6]),
            (vec![btreeset! {0, 1, 2}, btreeset! {2, 3, 4}], vec![5, 6]),
        ];

        for (case_id, (quorum_set, learners)) in cases.into_iter().enumerate() {
            for seed in 0..32 {
                let mut seed = seed + 1;
                let mut progress =
                    VecProgress::<(u64, u64), _>::new(quorum_set.clone(), learners.clone(), |id| (id, 0));

                assert_matches_model(&progress, &format!("case-{case_id} seed-{seed} initial"));

                for step in 0..128 {
                    let id = next_random(&mut seed) % 8;
                    let value =
                        progress.try_get(&id).map(|entry| entry.1).unwrap_or_default() + next_random(&mut seed) % 7 + 1;
                    let got = copy_result(progress.update(&id, value));
                    let want = model_quorum_accepted(&progress.quorum_set, &progress.entries);
                    let want_result = progress.try_get(&id).map(|_| want).ok_or(want);
                    let context = format!("case-{case_id} seed-{seed} step-{step} update-{id}-{value}");

                    assert_eq!(want_result, got, "{context}: entries: {:?}", progress.entries);
                    assert_matches_model(&progress, &context);
                }
            }
        }
    }

    #[test]
    fn vec_progress_matches_reference_model_after_random_quorum_upgrades() {
        let quorum_sets = [
            vec![btreeset! {0, 1, 2, 3, 4}],
            vec![btreeset! {0, 1, 2}, btreeset! {2, 3, 4}],
            vec![btreeset! {2, 3, 4}],
            vec![btreeset! {1, 3, 5}, btreeset! {3, 4, 5, 6}],
            vec![btreeset! {0, 5, 6}],
        ];
        let known_ids = (0..=8).collect::<Vec<_>>();

        for seed in 0..16 {
            let mut seed = seed + 11;
            let quorum_set = quorum_sets[0].clone();
            let learner_ids = learner_ids_for(&quorum_set, known_ids.clone());
            let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, learner_ids, |id| (id, 0));

            assert_matches_model(&progress, &format!("seed-{seed} initial"));

            for round in 0..24 {
                for step in 0..16 {
                    let id = next_random(&mut seed) % 10;
                    let value = progress.try_get(&id).map(|entry| entry.1).unwrap_or_default()
                        + next_random(&mut seed) % 11
                        + 1;
                    progress.update(&id, value).ok();
                    assert_matches_model(&progress, &format!("seed-{seed} round-{round} step-{step} update"));
                }

                let quorum_index = next_random(&mut seed) as usize % quorum_sets.len();
                let quorum_set = quorum_sets[quorum_index].clone();
                let learner_ids = learner_ids_for(&quorum_set, known_ids.clone());

                progress = progress.upgrade_quorum_set(quorum_set, learner_ids, |id| (id, 0));
                assert_matches_model(&progress, &format!("seed-{seed} round-{round} upgrade-{quorum_index}"));
            }
        }
    }

    #[test]
    fn vec_progress_joint_quorum_update() {
        let quorum_set = vec![btreeset! {0, 1, 2}, btreeset! {2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [5, 6], |id| (id, 0));

        let cases = [
            (0, 5, 0),
            (1, 5, 0),
            (2, 4, 0),
            (3, 4, 4),
            (4, 6, 4),
            (3, 6, 5),
            (2, 7, 5),
            (0, 7, 6),
        ];

        for (ith, (id, value, want_quorum_accepted)) in cases.iter().enumerate() {
            let got = copy_result(progress.update(id, *value));
            let context = format!("{ith}-th case: id:{id}, value:{value}");

            assert_eq!(
                Ok(*want_quorum_accepted),
                got,
                "{context}: entries: {:?}",
                progress.entries
            );
            assert_matches_model(&progress, &context);
        }

        let entries: Vec<_> = progress.collect_mapped(|item| (item.0, item.1));
        assert_eq!(vec![(2, 7), (0, 7), (4, 6), (3, 6), (1, 5), (5, 0), (6, 0)], entries);
    }

    #[test]
    fn vec_progress_non_member_and_learner_edge_cases() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [1, 3, 3], |id| (id, 0));

        assert_eq!(vec![(0, 0), (1, 0), (2, 0), (1, 0), (3, 0), (3, 0),], progress.entries);
        assert_eq!(3, progress.voter_count);
        assert_eq!(Some(true), progress.is_voter(&1));
        assert_eq!(Some(false), progress.is_voter(&3));
        assert_eq!(None, progress.is_voter(&9));

        assert_eq!(Ok(0), copy_result(progress.update(&3, 7)));
        assert_eq!(
            vec![(0, 0), (1, 0), (2, 0), (1, 0), (3, 7), (3, 0),],
            progress.entries,
            "only the first duplicate learner is updated"
        );

        assert_eq!(Ok(0), copy_result(progress.update(&1, 5)));
        assert_eq!(
            vec![(1, 5), (0, 0), (2, 0), (1, 0), (3, 7), (3, 0),],
            progress.entries,
            "a learner ID that is also a voter resolves to the voter entry"
        );

        assert_eq!(Ok(4), copy_result(progress.update(&2, 4)));
        assert_eq!(vec![(1, 5), (2, 4), (0, 0), (1, 0), (3, 7), (3, 0),], progress.entries);

        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut no_learners = VecProgress::<(u64, u64), _>::new(quorum_set, [], |id| (id, 0));

        assert_eq!(vec![(0, 0), (1, 0), (2, 0)], no_learners.entries);
        assert_eq!(3, no_learners.voter_count);
        assert_eq!(Err(0), copy_result(no_learners.update(&9, 5)));
        assert_eq!(vec![(0, 0), (1, 0), (2, 0)], no_learners.entries);
    }

    #[test]
    fn vec_progress_custom_quorum_set() {
        let quorum_set = RequiredSetQuorum::new([0, 1, 2, 3], [0, 3]);
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [], |id| (id, 0));

        assert_eq!(Ok(0), copy_result(progress.update(&1, 10)));
        assert_eq!(Ok(0), copy_result(progress.update(&2, 9)));
        assert_eq!(Ok(0), copy_result(progress.update(&0, 8)));
        assert_matches_model(&progress, "custom quorum before required set is reached");

        assert_eq!(vec![(1, 10), (2, 9), (0, 8), (3, 0)], progress.entries);

        assert_eq!(Ok(7), copy_result(progress.update(&3, 7)));
        assert_eq!(&7, progress.quorum_accepted());
        assert_matches_model(&progress, "custom quorum reaches required set");

        assert_eq!(Ok(8), copy_result(progress.update(&3, 11)));
        assert_eq!(vec![(3, 11), (1, 10), (2, 9), (0, 8)], progress.entries);
        assert_matches_model(&progress, "custom quorum follows required set threshold");
    }

    #[test]
    fn vec_progress_update_with() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6], |id| (id, 0));

        // Test that update_with can use closures to modify values
        // Case 0: 0,2,0,0,0,0
        let got = progress.update_with(&1, |x| *x += 2);
        assert_eq!(Ok(&0), got, "case 0: id:1, +=2");

        // Case 1: 0,2,3,0,0,0
        let got = progress.update_with(&2, |x| *x += 3);
        assert_eq!(Ok(&0), got, "case 1: id:2, +=3");

        // Case 2: 0,2,3,1,0,0
        let got = progress.update_with(&3, |x| *x = 1);
        assert_eq!(Ok(&1), got, "case 2: id:3, =1");

        // Case 3: 0,2,3,1,5,0
        let got = progress.update_with(&4, |x| *x += 5);
        assert_eq!(Ok(&2), got, "case 3: id:4, +5");

        // Case 4: 4,2,3,1,5,0 - closure can see updated value
        let got = progress.update_with(&0, |x| {
            *x += 4;
            assert_eq!(4, *x, "closure sees the updated value");
        });
        assert_eq!(Ok(&3), got, "case 4: id:0, +=4");

        // Case 5: 4,2,3,2,5,0 - using max
        let got = progress.update_with(&3, |x| *x = (*x).max(2));
        assert_eq!(Ok(&3), got, "case 5: id:3, max(2)");

        // Case 6: 4,4,3,2,5,0
        let got = progress.update_with(&1, |x| *x *= 2);
        assert_eq!(Ok(&4), got, "case 6: id:1, *=2");

        // Verify final values
        assert_eq!(&4, &progress.get(&0).1);
        assert_eq!(&4, &progress.get(&1).1);
        assert_eq!(&3, &progress.get(&2).1);
        assert_eq!(&2, &progress.get(&3).1);
        assert_eq!(&5, &progress.get(&4).1);
        assert_eq!(&0, &progress.get(&6).1);

        // Test nonexistent id returns Err with current quorum-accepted
        let got = progress.update_with(&9, |x| *x = 10);
        assert_eq!(Err(&4), got, "nonexistent id returns Err");
    }

    #[test]
    fn vec_progress_update_data_with() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<IdValData<u64, u64, &'static str>, _>::new(quorum_set, [3], |id| {
            IdValData::new(id, 0, "foo")
        });

        assert_eq!(Ok(&0), progress.update(&1, 2));

        let stats_before = (
            progress.stat().update_count,
            progress.stat().move_count,
            progress.stat().is_quorum_count,
        );

        assert_eq!(Some(&"bar"), progress.update_data_with(&1, |data| *data = "bar"));
        assert_eq!(None, progress.update_data_with(&9, |data| *data = "unknown"));

        assert_eq!(
            vec![
                IdValData::new(1, 2, "bar"),
                IdValData::new(0, 0, "foo"),
                IdValData::new(2, 0, "foo"),
                IdValData::new(3, 0, "foo"),
            ],
            progress.iter().cloned().collect::<Vec<_>>()
        );
        assert_eq!(&0, progress.quorum_accepted());
        assert_eq!(
            stats_before,
            (
                progress.stat().update_count,
                progress.stat().move_count,
                progress.stat().is_quorum_count,
            )
        );
    }

    #[test]
    fn vec_progress_update_does_not_move_learner_elt() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6], |id| (id, 0));

        assert_eq!(Some(5), progress.index(&6));

        progress.update(&6, 6).ok();
        assert_eq!(Some(5), progress.index(&6), "learner is not moved");

        progress.update(&4, 4).ok();
        assert_eq!(Some(0), progress.index(&4), "voter is not moved");
    }

    #[test]
    fn vec_progress_upgrade_quorum_set() {
        let qs012 = vec![btreeset! {0, 1, 2}];
        let qs012_345 = vec![btreeset! {0, 1, 2}, btreeset! {3, 4, 5}];
        let qs345 = vec![btreeset! {3, 4, 5}];

        // Initially, quorum-accepted is 5

        let mut p012 = VecProgress::<(u64, u64), _>::new(qs012, [5], |id| (id, 0));

        p012.update(&0, 5).ok();
        p012.update(&1, 6).ok();
        p012.update(&5, 9).ok();
        assert_eq!(&5, p012.quorum_accepted());

        // After upgrading to a bigger quorum set, quorum-accepted fall back to 0

        let mut p012_345 = p012.upgrade_quorum_set(qs012_345, [6], |id| (id, 0));
        assert_eq!(
            &0,
            p012_345.quorum_accepted(),
            "quorum extended from 012 to 012_345, quorum-accepted falls back"
        );
        assert_eq!(&9, &p012_345.get(&5).1, "inherit learner progress");

        // When quorum set shrinks, quorum-accepted becomes greater.

        p012_345.update(&3, 7).ok();
        p012_345.update(&4, 8).ok();
        assert_eq!(&5, p012_345.quorum_accepted());

        let p345 = p012_345.upgrade_quorum_set(qs345, [1], |id| (id, 0));

        assert_eq!(
            &8,
            p345.quorum_accepted(),
            "shrink quorum set, greater value becomes quorum-accepted"
        );
        assert_eq!(&6, &p345.get(&1).1, "inherit voter progress");
    }

    #[test]
    fn vec_progress_upgrade_joint_quorum_set() {
        let qs01234 = vec![btreeset! {0, 1, 2, 3, 4}];
        let qs012_234 = vec![btreeset! {0, 1, 2}, btreeset! {2, 3, 4}];
        let qs345 = vec![btreeset! {3, 4, 5}];

        let mut p = VecProgress::<(u64, u64), _>::new(qs01234, [5], |id| (id, 0));

        for (id, value) in [(0, 9), (1, 8), (2, 7), (3, 2), (4, 1), (5, 10)] {
            p.update(&id, value).ok();
        }

        assert_eq!(&7, p.quorum_accepted());

        let mut joint = p.upgrade_quorum_set(qs012_234, [5, 6], |id| (id, 0));

        assert_eq!(&2, joint.quorum_accepted(), "joint quorum lowers the accepted value");
        let entries: Vec<_> = joint.collect_mapped(|item| (item.0, item.1));
        assert_eq!(vec![(0, 9), (1, 8), (2, 7), (3, 2), (4, 1), (5, 10), (6, 0)], entries);
        assert_matches_model(&joint, "after upgrade to joint quorum");

        joint.update(&3, 8).ok();
        joint.update(&4, 8).ok();
        assert_eq!(&8, joint.quorum_accepted());
        assert_matches_model(&joint, "after joint quorum catches up");

        let shrunk = joint.upgrade_quorum_set(qs345, [0], |id| (id, 0));

        assert_eq!(&8, shrunk.quorum_accepted());
        let entries: Vec<_> = shrunk.collect_mapped(|item| (item.0, item.1));
        assert_eq!(vec![(5, 10), (3, 8), (4, 8), (0, 9)], entries);
        assert_matches_model(&shrunk, "after shrinking joint quorum");
    }

    #[test]
    fn vec_progress_is_voter() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6, 7], |id| (id, 0));

        assert_eq!(Some(true), progress.is_voter(&1));
        assert_eq!(Some(true), progress.is_voter(&3));
        assert_eq!(Some(false), progress.is_voter(&7));
        assert_eq!(None, progress.is_voter(&8));
    }

    #[test]
    fn vec_progress_display() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [3], |id| (id, 0));

        progress.update(&1, 5).ok();
        progress.update(&2, 3).ok();

        let display = format!(
            "{}",
            progress.display_with(|f, item| write!(f, "{}: {}", item.0, item.1))
        );
        assert_eq!("{1: 5, 2: 3, 0: 0, 3: 0}", display);
    }

    #[test]
    fn vec_progress_iter_mut_without_reorder() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [3], |id| (id, 0));

        // Mutate values through iter_mut_without_reorder
        for item in progress.iter_mut_without_reorder() {
            if item.0 == 1 {
                item.1 = 10;
            }
        }

        assert_eq!(&10, &progress.get(&1).1);
        assert_eq!(&0, &progress.get(&0).1);
        assert_eq!(&0, &progress.get(&2).1);
    }

    #[test]
    fn vec_progress_stat() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [3], |id| (id, 0));

        assert_eq!(
            (0, 0, 0),
            (
                progress.stat().update_count,
                progress.stat().move_count,
                progress.stat().is_quorum_count,
            )
        );

        progress.update(&3, 10).ok();
        assert_eq!(
            (1, 0, 0),
            (
                progress.stat().update_count,
                progress.stat().move_count,
                progress.stat().is_quorum_count,
            )
        );

        progress.update(&1, 5).ok();
        assert_eq!(
            (2, 1, 1),
            (
                progress.stat().update_count,
                progress.stat().move_count,
                progress.stat().is_quorum_count,
            )
        );

        progress.update(&2, 4).ok();
        assert_eq!(
            (3, 2, 2),
            (
                progress.stat().update_count,
                progress.stat().move_count,
                progress.stat().is_quorum_count,
            )
        );

        progress.update(&1, 6).ok();
        assert_eq!(
            (4, 3, 2),
            (
                progress.stat().update_count,
                progress.stat().move_count,
                progress.stat().is_quorum_count,
            )
        );

        progress.update(&9, 7).ok();
        assert_eq!(
            (5, 3, 2),
            (
                progress.stat().update_count,
                progress.stat().move_count,
                progress.stat().is_quorum_count,
            )
        );
    }

    #[test]
    fn vec_progress_display_with() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [3], |id| (id, 0));

        progress.update(&1, 5).ok();
        progress.update(&2, 3).ok();

        let display = progress.display_with(|f, item| write!(f, "{}={}", item.0, item.1));

        let output = format!("{}", display);
        assert_eq!("{1=5, 2=3, 0=0, 3=0}", output);
    }

    #[test]
    fn vec_progress_increase_to() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [6], |id| (id, 0));

        // Increase from 0 to 5
        progress.increase_to(&1, 5).ok();
        assert_eq!(&5, &progress.get(&1).1);

        // Try to decrease from 5 to 3 - should not change
        progress.increase_to(&1, 3).ok();
        assert_eq!(&5, &progress.get(&1).1);

        // Increase from 5 to 7
        progress.increase_to(&1, 7).ok();
        assert_eq!(&7, &progress.get(&1).1);

        // Try with nonexistent id
        let result = progress.increase_to(&9, 10);
        assert!(result.is_err());
    }

    #[test]
    fn vec_progress_collect_mapped() {
        let quorum_set = vec![btreeset! {0, 1, 2}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [3], |id| (id, 0));

        progress.update(&1, 5).ok();
        progress.update(&2, 3).ok();

        // Collect ids as Vec - order matters after updates (sorted by value descending)
        let ids: Vec<u64> = progress.collect_mapped(|item| item.0);
        assert_eq!(vec![1, 2, 0, 3], ids);

        // Collect values as Vec - order matters after updates (sorted descending)
        let values: Vec<u64> = progress.collect_mapped(|item| item.1);
        assert_eq!(vec![5, 3, 0, 0], values);

        // Collect as Vec of tuples - order matters after updates
        let pairs: Vec<(u64, u64)> = progress.collect_mapped(|item| (item.0, item.1));
        assert_eq!(vec![(1, 5), (2, 3), (0, 0), (3, 0)], pairs);
    }

    #[test]
    fn vec_progress_reset_entry_with() {
        // 7 voters, majority = 4.
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4, 5, 6}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [7], |id| (id, 0));

        progress.update(&0, 12).ok();
        progress.update(&1, 11).ok();
        progress.update(&2, 10).ok();
        progress.update(&3, 9).ok();
        assert_eq!(&9, progress.quorum_accepted());

        // Node 1 log-reverts: its progress falls back to the default value and the
        // entry is moved down, while the quorum-accepted value must be kept.
        let entry = progress.reset_entry_with(&1, |entry| entry.1 = 0);
        assert_eq!(Some(&(1, 0)), entry);
        assert_eq!(&9, progress.quorum_accepted(), "reset never lowers quorum-accepted");
        assert_eq!(
            vec![(0, 12), (2, 10), (3, 9), (1, 0), (4, 0), (5, 0), (6, 0), (7, 0)],
            progress.entries
        );
        assert_voter_prefix_is_sorted(&progress, "after reset");

        // Node 4 catches up to exactly 10: without the move-down, the reverted node 1
        // would be counted spuriously and 10 would be accepted with only 3 real grants.
        assert_eq!(Ok(9), copy_result(progress.update(&4, 10)));
        assert_matches_model(&progress, "after catching up to 10");

        // A real quorum at 10: {0, 2, 4, 5}.
        assert_eq!(Ok(10), copy_result(progress.update(&5, 10)));
        assert_matches_model(&progress, "after a real quorum at 10");

        // Resetting a learner or a nonexistent id does not reorder anything.
        assert_eq!(Some(&(7, 0)), progress.reset_entry_with(&7, |entry| entry.1 = 0));
        assert_eq!(None, progress.reset_entry_with(&9, |entry| entry.1 = 0));
    }

    #[test]
    fn vec_progress_matches_reference_model_with_resets() {
        let cases = [
            (vec![btreeset! {0, 1, 2, 3, 4, 5, 6}], vec![7]),
            (vec![btreeset! {0, 1, 2}, btreeset! {2, 3, 4}], vec![5, 6]),
        ];

        for (case_id, (quorum_set, learners)) in cases.into_iter().enumerate() {
            for seed in 0..32 {
                let mut seed = seed + 3;
                let mut progress =
                    VecProgress::<(u64, u64), _>::new(quorum_set.clone(), learners.clone(), |id| (id, 0));

                // The quorum-accepted value never moves backward, so the reference
                // is the running max of the instantaneous model value.
                let mut want = 0;

                for step in 0..128 {
                    // Use high bits: the low bits of this LCG have a short period,
                    // which makes power-of-two moduli cycle in lock-step.
                    let id = (next_random(&mut seed) >> 32) % 8;
                    let context = format!("case-{case_id} seed-{seed} step-{step} id-{id}");

                    if (next_random(&mut seed) >> 32).is_multiple_of(8) {
                        progress.reset_entry_with(&id, |entry| entry.1 = 0);
                    } else {
                        let value = progress.try_get(&id).map(|entry| entry.1).unwrap_or_default()
                            + next_random(&mut seed) % 7
                            + 1;
                        progress.update(&id, value).ok();
                    }

                    want = want.max(model_quorum_accepted(&progress.quorum_set, &progress.entries));
                    assert_eq!(
                        &want,
                        progress.quorum_accepted(),
                        "{context}: entries: {:?}",
                        progress.entries
                    );
                    assert_voter_prefix_is_sorted(&progress, &context);
                }
            }
        }
    }

    #[test]
    fn vec_progress_sub_quorum_commit_regression() {
        let quorum_set = vec![btreeset! {0, 1, 2, 3, 4}];
        let mut progress = VecProgress::<(u64, u64), _>::new(quorum_set, [], |id| (id, 0));

        progress.update(&0, 5).ok(); // qa = 0
        progress.update(&1, 3).ok(); // qa = 0
        progress.update(&2, 4).ok(); // qa = 3 ; above-qa region = [0:5, 2:4] (descending, ok)
        progress.update(&2, 10).ok(); // voter 2 was already > qa(3) and advances further:
        //   move_up must still run; if it were skipped, the region would become
        //   [0:5, 2:10] (NOT descending) and the next update would falsely accept 6.
        let qa = progress.update(&3, 6).ok();

        // True match values: {0:5, 1:3, 2:10, 3:6, 4:0}; sorted desc = 10,6,5,3,0 -> 3rd-largest = 5.
        // Only voters {2,3} reached 6 -> 2 voters -> NOT a majority.
        // Expected quorum_accepted = 5.
        assert_eq!(Some(&5), qa);
    }
}
