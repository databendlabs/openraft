//! Progress tracks replication state, i.e., it can be considered a map of node id to already replicated log id.
//!
//! A progress internally is a vector of scalar values.
//! The scalar value is monotonically incremental. Decreasing it is not allowed.
//! Optimization on calculating the committed log id is done on this assumption.

#[cfg(feature = "bench")]
#[cfg(test)]
mod bench;

use std::fmt::Debug;
use std::slice::Iter;

use crate::quorum::QuorumSet;

/// Track progress of several incremental values.
/// It calculates the committed value through a `QuorumSet`, when one of the value is updated.
pub(crate) trait Progress<ID, V, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    /// Update one of the scalar value and re-calculate the committed value.
    ///
    /// It returns Err(committed) if the id is not in this progress.
    fn update(&mut self, id: &ID, value: V) -> Result<&V, &V>;

    /// Get the value by `id`.
    fn get(&self, id: &ID) -> &V;

    /// Get the greatest value that is granted by a quorum defined in [`quorum_set()`].
    ///
    /// In raft or other distributed consensus,
    /// To commit a value, the value has to be **granted by a quorum** and has to be the greatest value
    /// every proposed.
    fn granted(&self) -> &V;

    /// Returns the reference to the quorum set
    fn quorum_set(&self) -> &QS;

    /// Iterate over all id and values, voters first followed by learners.
    fn iter(&self) -> Iter<(ID, V)>;

    /// Build a new instance with the new quorum set, inheriting progress data from `self`.
    fn upgrade_quorum_set(self, quorum_set: QS, learner_ids: &[ID]) -> Self;

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
pub(crate) struct VecProgress<ID, V, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    /// Quorum set to determine if a set of `id` constitutes a quorum, i.e., committed.
    quorum_set: QS,

    /// Currently already committed value.
    granted: V,

    /// Number of voters
    voter_count: usize,

    /// Progress data.
    ///
    /// Elements with values greater than the `granted` are sorted in descending order.
    /// Others are unsorted.
    ///
    /// The first `voter_count` elements are voters, the left are learners.
    /// Learner elements are always still.
    /// A voter element will be moved up to keep them in a descending order, when a new value is updated.
    vector: Vec<(ID, V)>,

    /// Statistics of how it runs.
    stat: Stat,
}

#[derive(Clone, Debug, Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct Stat {
    update_count: u64,
    move_count: u64,
    is_quorum_count: u64,
}

impl<ID, V, QS> VecProgress<ID, V, QS>
where
    ID: PartialEq + Copy + Debug + 'static,
    V: PartialOrd + Ord + Default + 'static,
    QS: QuorumSet<ID>,
{
    pub(crate) fn new(quorum_set: QS, learner_ids: impl Iterator<Item = ID>) -> Self {
        let mut vector = quorum_set.ids().map(|id| (id, V::default())).collect::<Vec<_>>();

        let voter_count = vector.len();

        vector.extend(learner_ids.map(|id| (id, V::default())));

        Self {
            quorum_set,
            granted: V::default(),
            voter_count,
            vector,
            stat: Default::default(),
        }
    }

    /// Find the index in of the specified id.
    #[inline(always)]
    fn index(&self, target: &ID) -> Option<usize> {
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
            if self.vector[i].1 < self.vector[i + 1].1 {
                self.vector.swap(i, i + 1);
            } else {
                return i + 1;
            }
        }

        0
    }

    #[allow(dead_code)]
    pub(crate) fn stat(&self) -> &Stat {
        &self.stat
    }
}

impl<ID, V, QS> Progress<ID, V, QS> for VecProgress<ID, V, QS>
where
    ID: PartialEq + Debug + Copy + 'static,
    V: PartialOrd + Ord + Copy + Default + 'static,
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
    /// and avoids unnecessary sorting: progresses are kept in order and only values greater than committed need to
    /// sort.
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
    fn update(&mut self, id: &ID, value: V) -> Result<&V, &V> {
        self.stat.update_count += 1;

        let index = match self.index(id) {
            None => {
                return Err(&self.granted);
            }
            Some(x) => x,
        };

        let elt = &mut self.vector[index];
        let prev = elt.1;

        if prev == value {
            return Ok(&self.granted);
        }

        debug_assert!(value > prev);

        elt.1 = value;

        // Learner does not grant a value.
        // And it won't be moved up to adjust the order.
        if index >= self.voter_count {
            return Ok(&self.granted);
        }

        if prev <= self.granted && self.granted < value {
            let new_index = self.move_up(index);

            // From high to low, find the max value that has constituted a quorum.
            for i in new_index..self.vector.len() {
                // No need to re-calculate already committed value.
                if self.vector[i].1 <= self.granted {
                    break;
                }

                // Ids of the target that has value GE `vector[i]`
                let it = self.vector[0..=i].iter().map(|x| &x.0);

                self.stat.is_quorum_count += 1;

                if self.quorum_set.is_quorum(it) {
                    self.granted = self.vector[i].1;
                    break;
                }
            }
        }

        Ok(&self.granted)
    }

    fn get(&self, id: &ID) -> &V {
        let index = self.index(id).unwrap();
        &self.vector[index].1
    }

    fn granted(&self) -> &V {
        &self.granted
    }

    fn quorum_set(&self) -> &QS {
        &self.quorum_set
    }

    fn iter(&self) -> Iter<(ID, V)> {
        self.vector.as_slice().iter()
    }

    fn upgrade_quorum_set(self, quorum_set: QS, leaner_ids: &[ID]) -> Self {
        let mut new_prog = Self::new(quorum_set, leaner_ids.iter().copied());

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
    use super::Progress;
    use super::VecProgress;
    use crate::quorum::Joint;

    #[test]
    fn vec_progress_new() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let progress = VecProgress::<u64, u64, _>::new(quorum_set, [6, 7].into_iter());

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
    fn vec_progress_iter() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, _>::new(quorum_set, [6, 7].into_iter());

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
        let mut progress = VecProgress::<u64, u64, _>::new(quorum_set, [6].into_iter());

        // initial: 0-0, 1-0, 2-0, 3-0, 4-0
        let cases = vec![
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
        let mut progress = VecProgress::<u64, u64, _>::new(quorum_set, [6].into_iter());

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

        for (ith, ((id, v), want_committed)) in cases.iter().enumerate() {
            let got = progress.update(id, *v);
            assert_eq!(want_committed.clone(), got, "{}-th case: id:{}, v:{}", ith, id, v);
        }
        Ok(())
    }

    #[test]
    fn vec_progress_update_does_not_move_learner_elt() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let mut progress = VecProgress::<u64, u64, _>::new(quorum_set, [6].into_iter());

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

        let mut p012 = VecProgress::<u64, u64, _>::new(qs012, [5].into_iter());

        let _ = p012.update(&0, 5);
        let _ = p012.update(&1, 6);
        let _ = p012.update(&5, 9);
        assert_eq!(&5, p012.granted());

        // After upgrading to a bigger quorum set, committed fall back to 0

        let mut p012_345 = p012.upgrade_quorum_set(qs012_345, &[6]);
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

        let p345 = p012_345.upgrade_quorum_set(qs345, &[1]);
        println!("{:?}", p345.vector);

        assert_eq!(&8, p345.granted(), "shrink quorum set, greater value becomes committed");
        assert_eq!(&6, p345.get(&1), "inherit voter progress");

        Ok(())
    }

    #[test]
    fn vec_progress_is_voter() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2, 3, 4];
        let progress = VecProgress::<u64, u64, _>::new(quorum_set, [6, 7].into_iter());

        assert_eq!(Some(true), progress.is_voter(&1));
        assert_eq!(Some(true), progress.is_voter(&3));
        assert_eq!(Some(false), progress.is_voter(&7));
        assert_eq!(None, progress.is_voter(&8));

        Ok(())
    }
}
