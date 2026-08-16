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

    /// Return the number of voters.
    pub(crate) fn voter_count(&self) -> usize {
        self.voter_count
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

    pub(crate) fn quorum_set(&self) -> &QS {
        &self.quorum_set
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
mod vec_progress_test;
