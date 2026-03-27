use std::collections::vec_deque;
use std::iter::Peekable;

use itertools::Itertools;

use crate::base::range_map::RangeMap;
use crate::base::range_map::RangeMapBound;
use crate::base::range_map::RangeMapValue;
use crate::base::range_values::RangeValues;

/// Iterator that yields [`RangeValues`] by scanning cursors across all range maps
/// at each merged boundary position.
///
/// `N` is the number of range maps. Internally holds a K-way merged boundary
/// stream (via `kmerge` + `dedup`) and per-range-map peekable iterator cursors.
/// At each boundary, cursors advance and a segment is emitted with the
/// current values from all range maps.
pub struct SegmentIter<'a, Bound, V, const N: usize>
where
    Bound: RangeMapBound,
    V: RangeMapValue,
{
    /// K-way merged, deduplicated boundary positions.
    merged_boundaries: Box<dyn Iterator<Item = Bound> + 'a>,

    /// Per-range-map peekable iterators yielding `(right_boundary, value)` pairs.
    right_value_iters: Vec<Peekable<vec_deque::Iter<'a, (Bound, V)>>>,

    /// Left boundary of the next segment to emit.
    left: Bound,
}

impl<'a, Bound, V, const N: usize> SegmentIter<'a, Bound, V, N>
where
    Bound: RangeMapBound + 'a,
    V: RangeMapValue,
{
    pub(crate) fn new(begin: Bound, range_maps: &'a [RangeMap<Bound, V>]) -> Self {
        debug_assert_eq!(range_maps.len(), N, "range_maps.len() must equal N");
        debug_assert!(!range_maps.is_empty(), "range_maps must be non-empty");

        let min_end = range_maps.iter().map(|rm| rm.end().unwrap_or(begin)).min().unwrap();

        let right_value_iters = range_maps.iter().map(|rm| rm.entries_after(begin).peekable()).collect::<Vec<_>>();

        let merged_boundaries: Box<dyn Iterator<Item = Bound> + 'a> = Box::new(
            range_maps
                .iter()
                .map(move |rm| rm.entries_after(begin).map(|&(k, _)| k))
                .kmerge()
                .dedup()
                .filter(move |&k| k <= min_end),
        );

        Self {
            merged_boundaries,
            right_value_iters,
            left: begin,
        }
    }
}

impl<Bound, V, const N: usize> Iterator for SegmentIter<'_, Bound, V, N>
where
    Bound: RangeMapBound,
    V: RangeMapValue,
{
    type Item = RangeValues<Bound, V, N>;

    fn next(&mut self) -> Option<Self::Item> {
        let right = self.merged_boundaries.next()?;

        let values = std::array::from_fn(|i| {
            let &&(entry_right, val) =
                self.right_value_iters[i].peek().expect("right_value_iter must have covering entry");
            if entry_right <= right {
                self.right_value_iters[i].next();
            }
            val
        });

        let segment = RangeValues {
            range: self.left..right,
            values,
        };

        self.left = right;
        Some(segment)
    }
}
