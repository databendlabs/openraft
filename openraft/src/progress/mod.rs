//! Progress tracks replication state, i.e., it can be considered a map of node id to already replicated log id.
//!
//! A progress internally is a vector of scalar values.
//! The scalar value is monotonically incremental. Decreasing it is not allowed.
//! Optimization on calculating the committed log id is done on this assumption.

#[cfg(feature = "bench")]
#[cfg(test)]
mod bench;

use std::fmt::Debug;

use crate::quorum::QuorumSet;

/// Track progress of several incremental values.
/// It calculates the committed value through a `QuorumSet`, when one of the value is updated.
pub(crate) trait Progress<ID, V, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    /// Update one of the scalar value and re-calculate the committed value.
    fn update(&mut self, id: &ID, value: V) -> &V;

    /// Get the value by `id`.
    fn get(&self, id: &ID) -> &V;

    /// Get the currently committed value.
    fn committed(&self) -> &V;
}

/// A Progress implementation with vector as storage.
///
/// Suitable for small quorum set
#[derive(Debug)]
pub(crate) struct VecProgress<ID, V, QS>
where
    ID: 'static,
    QS: QuorumSet<ID>,
{
    /// Quorum set to determine if a set of `id` constitutes a quorum, i.e., committed.
    quorum_set: QS,

    /// Currently already committed value.
    committed: V,

    /// Progress data.
    ///
    /// Elements with values greater than the `committed` are sorted in descending order.
    /// Others are unsorted.
    vector: Vec<(ID, V)>,

    /// Statistics of how it runs.
    stat: Stat,
}

#[derive(Debug, Default)]
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
    #[allow(dead_code)]
    pub(crate) fn new<'i>(ids: impl Iterator<Item = &'i ID>, quorum_set: QS) -> Self {
        let mut vector = Vec::new();
        for id in ids {
            vector.push((*id, V::default()));
        }

        Self {
            quorum_set,
            committed: V::default(),
            vector,
            stat: Default::default(),
        }
    }

    /// Find the index in of the specified id.
    #[inline(always)]
    fn index(&self, target: &ID) -> usize {
        for (i, elt) in self.vector.iter().enumerate() {
            if elt.0 == *target {
                return i;
            }
        }

        unreachable!("{:?} not found", target)
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
    fn update(&mut self, id: &ID, value: V) -> &V {
        self.stat.update_count += 1;

        let index = self.index(id);
        let elt = &mut self.vector[index];
        let prev = elt.1;

        if prev == value {
            return &self.committed;
        }

        debug_assert!(value > prev);

        elt.1 = value;

        if prev <= self.committed && self.committed < value {
            let new_index = self.move_up(index);

            // From high to low, find the max value that has constituted a quorum.
            for i in new_index..self.vector.len() {
                // No need to re-calculate already committed value.
                if self.vector[i].1 <= self.committed {
                    break;
                }

                // Ids of the target that has value GE `vector[i]`
                let it = self.vector[0..=i].iter().map(|x| &x.0);

                self.stat.is_quorum_count += 1;

                if self.quorum_set.is_quorum(it) {
                    self.committed = self.vector[i].1;
                    break;
                }
            }
        }

        &self.committed
    }

    fn get(&self, id: &ID) -> &V {
        let index = self.index(id);
        &self.vector[index].1
    }

    fn committed(&self) -> &V {
        &self.committed
    }
}

#[cfg(test)]
mod t {
    use super::Progress;
    use super::VecProgress;

    #[test]
    fn vec_progress_move_up() -> anyhow::Result<()> {
        let quorum_set: Vec<u64> = vec![0, 1, 2];
        let mut progress = VecProgress::<u64, u64, _>::new([0, 1, 2, 3, 4].iter(), quorum_set);

        // initial: 0-0, 1-0, 2-0, 3-0, 4-0
        let cases = vec![
            ((1, 2), &[(1, 2), (0, 0), (2, 0), (3, 0), (4, 0)], 0), //
            ((2, 3), &[(2, 3), (1, 2), (0, 0), (3, 0), (4, 0)], 0), //
            ((1, 3), &[(2, 3), (1, 3), (0, 0), (3, 0), (4, 0)], 1), // no move
            ((4, 8), &[(4, 8), (2, 3), (1, 3), (0, 0), (3, 0)], 0), //
            ((0, 5), &[(4, 8), (0, 5), (2, 3), (1, 3), (3, 0)], 1), // move to 1th
        ];
        for (ith, ((id, v), want_vec, want_new_index)) in cases.iter().enumerate() {
            // Update a value and move it up to keep the order.
            let index = progress.index(id);
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
        let mut progress = VecProgress::<u64, u64, _>::new([0, 1, 2, 3, 4].iter(), quorum_set);

        // initial: 0,0,0,0,0
        let cases = vec![
            ((1, 2), 0), // 0,2,0,0,0
            ((2, 3), 0), // 0,2,3,0,0
            ((3, 1), 1), // 0,2,3,1,0
            ((4, 5), 2), // 0,2,3,1,5
            ((0, 4), 3), // 4,2,3,1,5
            ((3, 2), 3), // 4,2,3,2,5
            ((3, 3), 3), // 4,2,3,2,5
            ((1, 4), 4), // 4,4,3,2,5
        ];

        for (ith, ((id, v), want_committed)) in cases.iter().enumerate() {
            let got = progress.update(id, *v);
            assert_eq!(want_committed, got, "{}-th case: id:{}, v:{}", ith, id, v);
        }
        Ok(())
    }
}
