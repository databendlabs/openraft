use std::borrow::Borrow;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::slice::Iter;
use std::slice::IterMut;

use super::Progress;
use super::display_vec_progress::DisplayVecProgress;
use super::id_val::IdVal;
use super::progress_stats::ProgressStats;
use crate::quorum::QuorumSet;

/// A Progress implementation with vector as storage.
///
/// Suitable for a small quorum set.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct VecProgress<ID, Ent, Prog, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    /// Quorum set to determine if a set of `id` constitutes a quorum.
    quorum_set: QS,

    /// The max value that is accepted by a quorum.
    quorum_accepted: Prog,

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
    entries: Vec<IdVal<ID, Ent>>,

    /// Statistics of how it runs.
    stat: ProgressStats,
}

impl<ID, Ent, Prog, QS> Display for VecProgress<ID, Ent, Prog, QS>
where
    ID: PartialEq + Debug + Clone + 'static,
    Ent: Clone + 'static,
    Ent: Borrow<Prog>,
    Prog: PartialOrd + Ord + Clone + 'static,
    QS: QuorumSet<ID> + 'static,
    ID: Display,
    Ent: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, item) in self.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", item)?
        }
        write!(f, "}}")?;

        Ok(())
    }
}

impl<ID, Ent, Prog, QS> VecProgress<ID, Ent, Prog, QS>
where
    ID: 'static,
    Ent: Borrow<Prog>,
    QS: QuorumSet<ID>,
    Prog: Clone,
{
    pub(crate) fn new(quorum_set: QS, learner_ids: impl IntoIterator<Item = ID>, default_v: impl Fn() -> Ent) -> Self {
        let mut entries = quorum_set.ids().map(|id| IdVal::new(id, default_v())).collect::<Vec<_>>();

        let voter_count = entries.len();

        entries.extend(learner_ids.into_iter().map(|id| IdVal::new(id, default_v())));

        Self {
            quorum_set,
            quorum_accepted: default_v().borrow().clone(),
            voter_count,
            entries,
            stat: Default::default(),
        }
    }

    /// Find the index of the specified id.
    #[inline(always)]
    pub(crate) fn index(&self, target: &ID) -> Option<usize>
    where ID: PartialEq {
        self.entries.iter().position(|item| &item.id == target)
    }

    /// Move an element at `index` up so that all the values greater than `quorum_accepted` are
    /// sorted.
    #[inline(always)]
    fn move_up(&mut self, index: usize) -> usize
    where Prog: PartialOrd {
        self.stat.move_count += 1;
        for i in (0..index).rev() {
            if self.entries[i].val.borrow() < self.entries[i + 1].val.borrow() {
                self.entries.swap(i, i + 1);
            } else {
                return i + 1;
            }
        }

        0
    }

    pub(crate) fn iter_mut(&mut self) -> IterMut<'_, IdVal<ID, Ent>> {
        self.entries.iter_mut()
    }

    pub(crate) fn into_iter(self) -> impl Iterator<Item = IdVal<ID, Ent>> {
        self.entries.into_iter()
    }

    #[allow(dead_code)]
    pub(crate) fn stat(&self) -> &ProgressStats {
        &self.stat
    }

    pub(crate) fn display_with<Fmt>(&self, f: Fmt) -> DisplayVecProgress<'_, ID, Ent, Prog, QS, Fmt>
    where Fmt: Fn(&mut Formatter<'_>, &ID, &Ent) -> std::fmt::Result {
        DisplayVecProgress { inner: self, f }
    }
}

impl<ID, Ent, Prog, QS> Progress<ID, Ent, Prog, QS> for VecProgress<ID, Ent, Prog, QS>
where
    ID: PartialEq + 'static,
    Ent: Borrow<Prog>,
    Prog: PartialOrd + Clone,
    QS: QuorumSet<ID>,
{
    /// Update one of the scalar values and re-calculate the quorum-accepted value.
    ///
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
    fn update_with<F>(&mut self, id: &ID, f: F) -> Result<&Prog, &Prog>
    where
        F: FnOnce(&mut Ent),
        ID: PartialEq,
    {
        self.stat.update_count += 1;

        let index = match self.index(id) {
            None => {
                return Err(&self.quorum_accepted);
            }
            Some(x) => x,
        };

        let ent = &mut self.entries[index];

        let prev_progress = ent.val.borrow().clone();

        f(&mut ent.val);

        let new_progress = ent.val.borrow();

        debug_assert!(new_progress >= &prev_progress,);

        // No change, return early
        if &prev_progress == new_progress {
            return Ok(&self.quorum_accepted);
        }

        // Learner does not grant a value.
        // And it won't be moved up to adjust the order.
        if index >= self.voter_count {
            return Ok(&self.quorum_accepted);
        }

        let prev_le_qa = prev_progress <= self.quorum_accepted;
        let new_gt_qa = new_progress > &self.quorum_accepted;

        // Sort and find the greatest value accepted by a quorum set.

        if prev_le_qa && new_gt_qa {
            let new_index = self.move_up(index);

            // From high to low, find the max value that has constituted a quorum.
            for i in new_index..self.voter_count {
                let prog = self.entries[i].val.borrow();

                // No need to re-calculate already quorum-accepted value.
                if prog <= &self.quorum_accepted {
                    break;
                }

                // Ids of the target that has value GE `entries[i]`
                let it = self.entries[0..=i].iter().map(|item| &item.id);

                self.stat.is_quorum_count += 1;

                if self.quorum_set.is_quorum(it) {
                    self.quorum_accepted = prog.clone();
                    break;
                }
            }
        }

        Ok(&self.quorum_accepted)
    }

    #[allow(dead_code)]
    fn try_get(&self, id: &ID) -> Option<&Ent> {
        let index = self.index(id)?;
        Some(&self.entries[index].val)
    }

    fn get_mut(&mut self, id: &ID) -> Option<&mut Ent> {
        let index = self.index(id)?;
        Some(&mut self.entries[index].val)
    }

    #[allow(dead_code)]
    fn get(&self, id: &ID) -> &Ent {
        let index = self.index(id).unwrap();
        &self.entries[index].val
    }

    #[allow(dead_code)]
    fn quorum_accepted(&self) -> &Prog {
        &self.quorum_accepted
    }

    #[allow(dead_code)]
    fn quorum_set(&self) -> &QS {
        &self.quorum_set
    }

    fn iter(&self) -> Iter<'_, IdVal<ID, Ent>> {
        self.entries.as_slice().iter()
    }

    fn upgrade_quorum_set(
        self,
        quorum_set: QS,
        learner_ids: impl IntoIterator<Item = ID>,
        default_v: impl Fn() -> Ent,
    ) -> Self {
        let mut new_prog = Self::new(quorum_set, learner_ids, default_v);

        new_prog.stat = self.stat.clone();

        for item in self.into_iter() {
            let _ = new_prog.update(&item.id, item.val);
        }
        new_prog
    }

    fn is_voter(&self, id: &ID) -> Option<bool> {
        let index = self.index(id)?;
        Some(index < self.voter_count)
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Borrow;

    use super::Progress;
    use super::VecProgress;
    use crate::progress::id_val::IdVal;
    use crate::quorum::Joint;

    #[test]
    fn vec_progress_new() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], || 0);

        assert_eq!(
            vec![
                IdVal::new(0, 0),
                IdVal::new(1, 0),
                IdVal::new(2, 0),
                IdVal::new(3, 0),
                IdVal::new(4, 0),
                IdVal::new(6, 0),
                IdVal::new(7, 0),
            ],
            progress.entries
        );
        assert_eq!(5, progress.voter_count);
    }

    #[test]
    fn vec_progress_index() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], || 0);

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
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], || 0);

        let _ = progress.update(&6, 5);
        assert_eq!(&5, progress.get(&6));
        assert_eq!(Some(&5), progress.try_get(&6));
        assert_eq!(None, progress.try_get(&9));

        {
            let x = progress.get_mut(&6);
            if let Some(x) = x {
                *x = 10;
            }
        }
        assert_eq!(Some(&10), progress.try_get(&6));
    }

    #[test]
    fn vec_progress_iter() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], || 0);

        let _ = progress.update(&7, 7);
        let _ = progress.update(&3, 3);
        let _ = progress.update(&1, 1);

        assert_eq!(
            vec![
                IdVal::new(3, 3),
                IdVal::new(1, 1),
                IdVal::new(0, 0),
                IdVal::new(2, 0),
                IdVal::new(4, 0),
                IdVal::new(6, 0),
                IdVal::new(7, 7),
            ],
            progress.iter().cloned().collect::<Vec<_>>(),
            "iter() returns voter first, followed by learners"
        );
    }

    #[test]
    fn vec_progress_move_up() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], || 0);

        // initial: 0-0, 1-0, 2-0, 3-0, 4-0
        let cases = [
            (
                (1, 2),
                vec![
                    IdVal::new(1, 2),
                    IdVal::new(0, 0),
                    IdVal::new(2, 0),
                    IdVal::new(3, 0),
                    IdVal::new(4, 0),
                    IdVal::new(6, 0),
                ],
                0,
            ),
            (
                (2, 3),
                vec![
                    IdVal::new(2, 3),
                    IdVal::new(1, 2),
                    IdVal::new(0, 0),
                    IdVal::new(3, 0),
                    IdVal::new(4, 0),
                    IdVal::new(6, 0),
                ],
                0,
            ),
            (
                (1, 3),
                vec![
                    IdVal::new(2, 3),
                    IdVal::new(1, 3),
                    IdVal::new(0, 0),
                    IdVal::new(3, 0),
                    IdVal::new(4, 0),
                    IdVal::new(6, 0),
                ],
                1,
            ), // no move
            (
                (4, 8),
                vec![
                    IdVal::new(4, 8),
                    IdVal::new(2, 3),
                    IdVal::new(1, 3),
                    IdVal::new(0, 0),
                    IdVal::new(3, 0),
                    IdVal::new(6, 0),
                ],
                0,
            ),
            (
                (0, 5),
                vec![
                    IdVal::new(4, 8),
                    IdVal::new(0, 5),
                    IdVal::new(2, 3),
                    IdVal::new(1, 3),
                    IdVal::new(3, 0),
                    IdVal::new(6, 0),
                ],
                1,
            ), // move to 1st
        ];
        for (ith, ((id, v), want_vec, want_new_index)) in cases.iter().enumerate() {
            // Update a value and move it up to keep the order.
            let index = progress.index(id).unwrap();
            progress.entries[index].val = *v;
            let got = progress.move_up(index);

            assert_eq!(want_vec, &progress.entries, "{}-th case: idx:{}, v:{}", ith, *id, *v);
            assert_eq!(*want_new_index, got, "{}-th case: idx:{}, v:{}", ith, *id, *v);
        }
    }

    #[test]
    fn vec_progress_update() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], || 0);

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
    fn vec_progress_update_with() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], || 0);

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
        assert_eq!(&4, progress.get(&0));
        assert_eq!(&4, progress.get(&1));
        assert_eq!(&3, progress.get(&2));
        assert_eq!(&2, progress.get(&3));
        assert_eq!(&5, progress.get(&4));
        assert_eq!(&0, progress.get(&6));

        // Test nonexistent id returns Err with current quorum-accepted
        let got = progress.update_with(&9, |x| *x = 10);
        assert_eq!(Err(&4), got, "nonexistent id returns Err");
    }

    /// Progress entry for testing
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct ProgressEntry {
        progress: u64,
        user_data: &'static str,
    }

    impl Borrow<u64> for ProgressEntry {
        fn borrow(&self) -> &u64 {
            &self.progress
        }
    }

    #[test]
    fn vec_progress_update_struct_value() {
        let pv = |p, user_data| ProgressEntry { progress: p, user_data };

        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, ProgressEntry, u64, _>::new(quorum_set, [3], || pv(0, "foo"));

        // initial: 0,0,0,0
        let cases = [
            (3, pv(9, "a"), Ok(&0)), // 0,0,0,9 // learner won't affect quorum-accepted
            (1, pv(2, "b"), Ok(&0)), // 0,2,0,9
            (2, pv(3, "c"), Ok(&2)), // 0,2,3,9
            (1, pv(2, "d"), Ok(&2)), // 0,2,3,9 // No new quorum-accepted, just update user data.
        ];

        for (ith, (id, v, want_quorum_accepted)) in cases.iter().enumerate() {
            let got = progress.update(id, *v);
            assert_eq!(
                want_quorum_accepted.clone(),
                got,
                "{}-th case: id:{}, v:{:?}",
                ith,
                id,
                v
            );
        }

        // Check progress data

        assert_eq!(pv(0, "foo"), *progress.get(&0),);
        assert_eq!(pv(2, "d"), *progress.get(&1),);
        assert_eq!(pv(3, "c"), *progress.get(&2),);
        assert_eq!(pv(9, "a"), *progress.get(&3),);
    }

    #[test]
    fn vec_progress_update_does_not_move_learner_elt() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], || 0);

        assert_eq!(Some(5), progress.index(&6));

        let _ = progress.update(&6, 6);
        assert_eq!(Some(5), progress.index(&6), "learner is not moved");

        let _ = progress.update(&4, 4);
        assert_eq!(Some(0), progress.index(&4), "voter is not moved");
    }

    #[test]
    fn vec_progress_upgrade_quorum_set() {
        let qs012 = Joint::from(vec![vec![0, 1, 2]]);
        let qs012_345 = Joint::from(vec![vec![0, 1, 2], vec![3, 4, 5]]);
        let qs345 = Joint::from(vec![vec![3, 4, 5]]);

        // Initially, quorum-accepted is 5

        let mut p012 = VecProgress::<u64, u64, u64, _>::new(qs012, [5], || 0);

        let _ = p012.update(&0, 5);
        let _ = p012.update(&1, 6);
        let _ = p012.update(&5, 9);
        assert_eq!(&5, p012.quorum_accepted());

        // After upgrading to a bigger quorum set, quorum-accepted fall back to 0

        let mut p012_345 = p012.upgrade_quorum_set(qs012_345, [6], || 0);
        assert_eq!(
            &0,
            p012_345.quorum_accepted(),
            "quorum extended from 012 to 012_345, quorum-accepted falls back"
        );
        assert_eq!(&9, p012_345.get(&5), "inherit learner progress");

        // When quorum set shrinks, quorum-accepted becomes greater.

        let _ = p012_345.update(&3, 7);
        let _ = p012_345.update(&4, 8);
        assert_eq!(&5, p012_345.quorum_accepted());

        let p345 = p012_345.upgrade_quorum_set(qs345, [1], || 0);

        assert_eq!(
            &8,
            p345.quorum_accepted(),
            "shrink quorum set, greater value becomes quorum-accepted"
        );
        assert_eq!(&6, p345.get(&1), "inherit voter progress");
    }

    #[test]
    fn vec_progress_is_voter() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], || 0);

        assert_eq!(Some(true), progress.is_voter(&1));
        assert_eq!(Some(true), progress.is_voter(&3));
        assert_eq!(Some(false), progress.is_voter(&7));
        assert_eq!(None, progress.is_voter(&8));
    }

    #[test]
    fn vec_progress_display() {
        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [3], || 0);

        let _ = progress.update(&1, 5);
        let _ = progress.update(&2, 3);

        let display = format!("{}", progress);
        assert_eq!("{1: 5, 2: 3, 0: 0, 3: 0}", display);
    }

    #[test]
    fn vec_progress_iter_mut() {
        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [3], || 0);

        // Mutate values through iter_mut
        for item in progress.iter_mut() {
            if item.id == 1 {
                item.val = 10;
            }
        }

        assert_eq!(&10, progress.get(&1));
        assert_eq!(&0, progress.get(&0));
        assert_eq!(&0, progress.get(&2));
    }

    #[test]
    fn vec_progress_stat() {
        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [3], || 0);

        assert_eq!(0, progress.stat().update_count);
        assert_eq!(0, progress.stat().move_count);

        let _ = progress.update(&1, 5);
        assert_eq!(1, progress.stat().update_count);

        let _ = progress.update(&2, 3);
        assert_eq!(2, progress.stat().update_count);
        assert!(progress.stat().move_count > 0);
    }

    #[test]
    fn vec_progress_display_with() {
        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [3], || 0);

        let _ = progress.update(&1, 5);
        let _ = progress.update(&2, 3);

        let display = progress.display_with(|f, id, val| write!(f, "{}={}", id, val));

        let output = format!("{}", display);
        assert_eq!("{1=5, 2=3, 0=0, 3=0}", output);
    }

    #[test]
    fn vec_progress_increase_to() {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], || 0);

        // Increase from 0 to 5
        let _ = progress.increase_to(&1, 5);
        assert_eq!(&5, progress.get(&1));

        // Try to decrease from 5 to 3 - should not change
        let _ = progress.increase_to(&1, 3);
        assert_eq!(&5, progress.get(&1));

        // Increase from 5 to 7
        let _ = progress.increase_to(&1, 7);
        assert_eq!(&7, progress.get(&1));

        // Try with nonexistent id
        let result = progress.increase_to(&9, 10);
        assert!(result.is_err());
    }

    #[test]
    fn vec_progress_collect_mapped() {
        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [3], || 0);

        let _ = progress.update(&1, 5);
        let _ = progress.update(&2, 3);

        // Collect ids as Vec - order matters after updates (sorted by value descending)
        let ids: Vec<u64> = progress.collect_mapped(|item| item.id);
        assert_eq!(vec![1, 2, 0, 3], ids);

        // Collect values as Vec - order matters after updates (sorted descending)
        let values: Vec<u64> = progress.collect_mapped(|item| item.val);
        assert_eq!(vec![5, 3, 0, 0], values);

        // Collect as Vec of tuples - order matters after updates
        let pairs: Vec<(u64, u64)> = progress.collect_mapped(|item| (item.id, item.val));
        assert_eq!(vec![(1, 5), (2, 3), (0, 0), (3, 0)], pairs);
    }
}
