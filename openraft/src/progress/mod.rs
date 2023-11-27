//! Progress tracks replication state, i.e., it can be considered a map of node id to already
//! replicated log id.
//!
//! A progress internally is a vector of scalar values.
//! The scalar value is monotonically incremental. Decreasing it is not allowed.
//! Optimization on calculating the committed log id is done on this assumption.

#[cfg(feature = "bench")]
#[cfg(test)]
mod bench;
pub(crate) mod entry;
pub(crate) mod inflight;

use std::borrow::Borrow;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::slice::Iter;
use std::slice::IterMut;

// TODO: remove it
#[allow(unused_imports)] pub(crate) use inflight::Inflight;

use crate::quorum::QuorumSet;

/// Track progress of several incremental values.
///
/// When one of the value is updated, it uses a `QuorumSet` to calculate the committed value.
/// `ID` is the identifier of every progress value.
/// `V` is type of a progress entry.
/// `P` is the progress data of `V`, a progress entry `V` could contain other user data.
/// `QS` is a quorum set implementation.
pub(crate) trait Progress<ID, V, P, QS>
where
    ID: 'static,
    V: Borrow<P>,
    QS: QuorumSet<ID>,
{
    /// Update one of the scalar value and re-calculate the committed value with provided function.
    ///
    /// It returns Err(committed) if the `id` is not found.
    /// The provided function `f` update the value of `id`.
    fn update_with<F>(&mut self, id: &ID, f: F) -> Result<&P, &P>
    where F: FnOnce(&mut V);

    /// Update one of the scalar value and re-calculate the committed value.
    ///
    /// It returns Err(committed) if the `id` is not found.
    fn update(&mut self, id: &ID, value: V) -> Result<&P, &P> {
        self.update_with(id, |x| *x = value)
    }

    /// Try to get the value by `id`.
    fn try_get(&self, id: &ID) -> Option<&V>;

    /// Returns a mutable reference to the value corresponding to the `id`.
    fn get_mut(&mut self, id: &ID) -> Option<&mut V>;

    // TODO: merge `get` and `try_get`
    /// Get the value by `id`.
    fn get(&self, id: &ID) -> &V;

    /// Get the greatest value that is granted by a quorum defined in [`Self::quorum_set()`].
    ///
    /// In raft or other distributed consensus,
    /// To commit a value, the value has to be **granted by a quorum** and has to be the greatest
    /// value every proposed.
    fn granted(&self) -> &P;

    /// Returns the reference to the quorum set
    fn quorum_set(&self) -> &QS;

    /// Iterate over all id and values, voters first followed by learners.
    fn iter(&self) -> Iter<(ID, V)>;

    /// Build a new instance with the new quorum set, inheriting progress data from `self`.
    fn upgrade_quorum_set(self, quorum_set: QS, learner_ids: &[ID], default_v: V) -> Self;

    /// Return if the given id is a voter.
    ///
    /// A voter is a node in the quorum set that can grant a value.
    /// A learner's progress is also tracked but it will never grant a value.
    ///
    /// If the given id is not in this `Progress`, it returns `None`.
    fn is_voter(&self, id: &ID) -> Option<bool>;
}

/// A Progress implementation with vector as storage.
///
/// Suitable for small quorum set.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct VecProgress<ID, V, P, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    /// Quorum set to determine if a set of `id` constitutes a quorum, i.e., committed.
    quorum_set: QS,

    /// Currently already committed value.
    granted: P,

    /// Number of voters
    voter_count: usize,

    /// Progress data.
    ///
    /// Elements with values greater than the `granted` are sorted in descending order.
    /// Others are unsorted.
    ///
    /// The first `voter_count` elements are voters, the left are learners.
    /// Learner elements are always still.
    /// A voter element will be moved up to keep them in a descending order, when a new value is
    /// updated.
    vector: Vec<(ID, V)>,

    /// Statistics of how it runs.
    stat: Stat,
}

impl<ID, V, P, QS> Display for VecProgress<ID, V, P, QS>
where
    ID: PartialEq + Debug + Copy + 'static,
    V: Copy + 'static,
    V: Borrow<P>,
    P: PartialOrd + Ord + Copy + 'static,
    QS: QuorumSet<ID> + 'static,
    ID: Display,
    V: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, (id, v)) in self.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", id, v)?
        }
        write!(f, "}}")?;

        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct Stat {
    update_count: u64,
    move_count: u64,
    is_quorum_count: u64,
}

impl<ID, V, P, QS> VecProgress<ID, V, P, QS>
where
    ID: PartialEq + Copy + Debug + 'static,
    V: Copy + 'static,
    V: Borrow<P>,
    P: PartialOrd + Ord + Copy + 'static,
    QS: QuorumSet<ID>,
{
    pub(crate) fn new(quorum_set: QS, learner_ids: impl IntoIterator<Item = ID>, default_v: V) -> Self {
        let mut vector = quorum_set.ids().map(|id| (id, default_v)).collect::<Vec<_>>();

        let voter_count = vector.len();

        vector.extend(learner_ids.into_iter().map(|id| (id, default_v)));

        Self {
            quorum_set,
            granted: *default_v.borrow(),
            voter_count,
            vector,
            stat: Default::default(),
        }
    }

    /// Find the index in of the specified id.
    #[inline(always)]
    pub(crate) fn index(&self, target: &ID) -> Option<usize> {
        for (i, elt) in self.vector.iter().enumerate() {
            if elt.0 == *target {
                return Some(i);
            }
        }

        None
    }

    /// Move an element at `index` up so that all the values greater than `committed` are sorted.
    #[inline(always)]
    fn move_up(&mut self, index: usize) -> usize {
        self.stat.move_count += 1;
        for i in (0..index).rev() {
            if self.vector[i].1.borrow() < self.vector[i + 1].1.borrow() {
                self.vector.swap(i, i + 1);
            } else {
                return i + 1;
            }
        }

        0
    }

    pub(crate) fn iter_mut(&mut self) -> IterMut<(ID, V)> {
        self.vector.iter_mut()
    }

    #[allow(dead_code)]
    pub(crate) fn stat(&self) -> &Stat {
        &self.stat
    }
}

impl<ID, V, P, QS> Progress<ID, V, P, QS> for VecProgress<ID, V, P, QS>
where
    ID: PartialEq + Debug + Copy + 'static,
    V: Copy + 'static,
    V: Borrow<P>,
    P: PartialOrd + Ord + Copy + 'static,
    QS: QuorumSet<ID> + 'static,
{
    /// Update one of the scalar value and re-calculate the committed value.
    ///
    /// Re-updating with a same V will do nothing.
    ///
    /// # Algorithm
    ///
    /// Only when the **previous value** is less or equal the committed,
    /// and the **new value** is greater than the committed,
    /// there is possibly an update to the committed.
    ///
    /// This way it gets rid of a portion of unnecessary re-calculation of committed,
    /// and avoids unnecessary sorting: progresses are kept in order and only values greater than
    /// committed need to sort.
    ///
    /// E.g., given 3 ids with value `1,3,5`,
    /// the committed is `3` and assumes a majority quorum set is used.
    /// Then:
    /// - update(a, 6): nothing to do: committed is still 3;
    /// - update(b, 4): re-calc:       committed becomes 4;
    /// - update(b, 6): re-calc:       committed becomes 5;
    /// - update(c, 2): nothing to do: committed is still 3;
    /// - update(c, 3): nothing to do: committed is still 3;
    /// - update(c, 4): re-calc:       committed becomes 4;
    /// - update(c, 6): re-calc:       committed becomes 5;
    ///
    /// ```text
    /// a -----------+-------->
    /// b -------+------------>
    /// c ---+---------------->
    /// ------------------------------
    ///      1   3   5
    /// ```
    fn update_with<F>(&mut self, id: &ID, f: F) -> Result<&P, &P>
    where F: FnOnce(&mut V) {
        self.stat.update_count += 1;

        let index = match self.index(id) {
            None => {
                return Err(&self.granted);
            }
            Some(x) => x,
        };

        let elt = &mut self.vector[index];

        let prev_progress = *elt.1.borrow();

        f(&mut elt.1);

        let new_progress = elt.1.borrow();

        debug_assert!(new_progress >= &prev_progress,);

        let prev_le_granted = prev_progress <= self.granted;
        let new_gt_granted = new_progress > &self.granted;

        if &prev_progress == new_progress {
            return Ok(&self.granted);
        }

        // Learner does not grant a value.
        // And it won't be moved up to adjust the order.
        if index >= self.voter_count {
            return Ok(&self.granted);
        }

        // Sort and find the greatest value granted by a quorum set.

        if prev_le_granted && new_gt_granted {
            let new_index = self.move_up(index);

            // From high to low, find the max value that has constituted a quorum.
            for i in new_index..self.voter_count {
                let prog = self.vector[i].1.borrow();

                // No need to re-calculate already committed value.
                if prog <= &self.granted {
                    break;
                }

                // Ids of the target that has value GE `vector[i]`
                let it = self.vector[0..=i].iter().map(|x| &x.0);

                self.stat.is_quorum_count += 1;

                if self.quorum_set.is_quorum(it) {
                    self.granted = *prog;
                    break;
                }
            }
        }

        Ok(&self.granted)
    }

    fn try_get(&self, id: &ID) -> Option<&V> {
        let index = self.index(id)?;
        Some(&self.vector[index].1)
    }

    fn get_mut(&mut self, id: &ID) -> Option<&mut V> {
        let index = self.index(id)?;
        Some(&mut self.vector[index].1)
    }

    fn get(&self, id: &ID) -> &V {
        let index = self.index(id).unwrap();
        &self.vector[index].1
    }

    fn granted(&self) -> &P {
        &self.granted
    }

    fn quorum_set(&self) -> &QS {
        &self.quorum_set
    }

    fn iter(&self) -> Iter<(ID, V)> {
        self.vector.as_slice().iter()
    }

    fn upgrade_quorum_set(self, quorum_set: QS, leaner_ids: &[ID], default_v: V) -> Self {
        let mut new_prog = Self::new(quorum_set, leaner_ids.iter().copied(), default_v);

        new_prog.stat = self.stat.clone();

        for (id, v) in self.iter() {
            let _ = new_prog.update(id, *v);
        }
        new_prog
    }

    fn is_voter(&self, id: &ID) -> Option<bool> {
        let index = self.index(id)?;
        Some(index < self.voter_count)
    }
}

#[cfg(test)]
mod t {
    use std::borrow::Borrow;

    use super::Progress;
    use super::VecProgress;
    use crate::quorum::Joint;

    #[test]
    fn vec_progress_new() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], 0);

        assert_eq!(
            vec![
                //
                (0, 0),
                (1, 0),
                (2, 0),
                (3, 0),
                (4, 0),
                (6, 0),
                (7, 0),
            ],
            progress.vector
        );
        assert_eq!(5, progress.voter_count);

        Ok(())
    }

    #[test]
    fn vec_progress_get() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], 0);

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

        Ok(())
    }

    #[test]
    fn vec_progress_iter() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], 0);

        let _ = progress.update(&7, 7);
        let _ = progress.update(&3, 3);
        let _ = progress.update(&1, 1);

        assert_eq!(
            vec![
                //
                (3, 3),
                (1, 1),
                (0, 0),
                (2, 0),
                (4, 0),
                (6, 0),
                (7, 7),
            ],
            progress.iter().copied().collect::<Vec<_>>(),
            "iter() returns voter first, followed by learners"
        );

        Ok(())
    }

    #[test]
    fn vec_progress_move_up() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], 0);

        // initial: 0-0, 1-0, 2-0, 3-0, 4-0
        let cases = [
            ((1, 2), &[(1, 2), (0, 0), (2, 0), (3, 0), (4, 0), (6, 0)], 0), //
            ((2, 3), &[(2, 3), (1, 2), (0, 0), (3, 0), (4, 0), (6, 0)], 0), //
            ((1, 3), &[(2, 3), (1, 3), (0, 0), (3, 0), (4, 0), (6, 0)], 1), // no move
            ((4, 8), &[(4, 8), (2, 3), (1, 3), (0, 0), (3, 0), (6, 0)], 0), //
            ((0, 5), &[(4, 8), (0, 5), (2, 3), (1, 3), (3, 0), (6, 0)], 1), // move to 1th
        ];
        for (ith, ((id, v), want_vec, want_new_index)) in cases.iter().enumerate() {
            // Update a value and move it up to keep the order.
            let index = progress.index(id).unwrap();
            progress.vector[index].1 = *v;
            let got = progress.move_up(index);

            assert_eq!(
                want_vec.as_slice(),
                &progress.vector,
                "{}-th case: idx:{}, v:{}",
                ith,
                *id,
                *v
            );
            assert_eq!(*want_new_index, got, "{}-th case: idx:{}, v:{}", ith, *id, *v);
        }
        Ok(())
    }

    #[test]
    fn vec_progress_update() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], 0);

        // initial: 0,0,0,0,0
        let cases = vec![
            ((6, 9), Ok(&0)),  // 0,0,0,0,0,9 // learner won't affect granted
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

        // TODO: test update_with
        for (ith, ((id, v), want_committed)) in cases.iter().enumerate() {
            let got = progress.update_with(id, |x| *x = *v);
            assert_eq!(want_committed.clone(), got, "{}-th case: id:{}, v:{}", ith, id, v);
        }
        Ok(())
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
    fn vec_progress_update_struct_value() -> anyhow::Result<()> {
        let pv = |p, user_data| ProgressEntry { progress: p, user_data };

        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, ProgressEntry, u64, _>::new(quorum_set, [3], pv(0, "foo"));

        // initial: 0,0,0,0
        let cases = [
            (3, pv(9, "a"), Ok(&0)), // 0,0,0,9 // learner won't affect granted
            (1, pv(2, "b"), Ok(&0)), // 0,2,0,9
            (2, pv(3, "c"), Ok(&2)), // 0,2,3,9
            (1, pv(2, "d"), Ok(&2)), // 0,2,3,9 // No new granted, just update user data.
        ];

        for (ith, (id, v, want_committed)) in cases.iter().enumerate() {
            let got = progress.update(id, *v);
            assert_eq!(want_committed.clone(), got, "{}-th case: id:{}, v:{:?}", ith, id, v);
        }

        // Check progress data

        assert_eq!(pv(0, "foo"), *progress.get(&0),);
        assert_eq!(pv(2, "d"), *progress.get(&1),);
        assert_eq!(pv(3, "c"), *progress.get(&2),);
        assert_eq!(pv(9, "a"), *progress.get(&3),);

        Ok(())
    }

    #[test]
    fn vec_progress_update_does_not_move_learner_elt() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6], 0);

        assert_eq!(Some(5), progress.index(&6));

        let _ = progress.update(&6, 6);
        assert_eq!(Some(5), progress.index(&6), "learner is not moved");

        let _ = progress.update(&4, 4);
        assert_eq!(Some(0), progress.index(&4), "voter is not moved");
        Ok(())
    }

    #[test]
    fn vec_progress_upgrade_quorum_set() -> anyhow::Result<()> {
        let qs012 = Joint::from(vec![vec![0, 1, 2]]);
        let qs012_345 = Joint::from(vec![vec![0, 1, 2], vec![3, 4, 5]]);
        let qs345 = Joint::from(vec![vec![3, 4, 5]]);

        // Initially, committed is 5

        let mut p012 = VecProgress::<u64, u64, u64, _>::new(qs012, [5], 0);

        let _ = p012.update(&0, 5);
        let _ = p012.update(&1, 6);
        let _ = p012.update(&5, 9);
        assert_eq!(&5, p012.granted());

        // After upgrading to a bigger quorum set, committed fall back to 0

        let mut p012_345 = p012.upgrade_quorum_set(qs012_345, &[6], 0);
        assert_eq!(
            &0,
            p012_345.granted(),
            "quorum extended from 012 to 012_345, committed falls back"
        );
        assert_eq!(&9, p012_345.get(&5), "inherit learner progress");

        // When quorum set shrinks, committed becomes greater.

        let _ = p012_345.update(&3, 7);
        let _ = p012_345.update(&4, 8);
        assert_eq!(&5, p012_345.granted());

        let p345 = p012_345.upgrade_quorum_set(qs345, &[1], 0);

        assert_eq!(&8, p345.granted(), "shrink quorum set, greater value becomes committed");
        assert_eq!(&6, p345.get(&1), "inherit voter progress");

        Ok(())
    }

    #[test]
    fn vec_progress_is_voter() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let progress = VecProgress::<u64, u64, u64, _>::new(quorum_set, [6, 7], 0);

        assert_eq!(Some(true), progress.is_voter(&1));
        assert_eq!(Some(true), progress.is_voter(&3));
        assert_eq!(Some(false), progress.is_voter(&7));
        assert_eq!(None, progress.is_voter(&8));

        Ok(())
    }
}
