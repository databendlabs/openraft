use std::collections::vec_deque;
use std::iter::Peekable;
use std::ops::Range;

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
/// current values from all range maps, unless the segment lies in an excluded range.
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

    /// Sorted, disjoint ranges whose segments are not yielded.
    ///
    /// Their edges are among the merged boundaries, so a segment lies inside one of them or outside
    /// all of them.
    excluded: &'a [Range<Bound>],
}

impl<'a, Bound, V, const N: usize> SegmentIter<'a, Bound, V, N>
where
    Bound: RangeMapBound + 'a,
    V: RangeMapValue,
{
    /// Iterate the segments from `begin` on, skipping those in `excluded`, which must be sorted
    /// and disjoint.
    pub(crate) fn new(begin: Bound, range_maps: &'a [RangeMap<Bound, V>], excluded: &'a [Range<Bound>]) -> Self {
        debug_assert_eq!(range_maps.len(), N, "range_maps.len() must equal N");
        debug_assert!(!range_maps.is_empty(), "range_maps must be non-empty");
        debug_assert!(
            excluded.windows(2).all(|w| w[0].end <= w[1].start),
            "excluded ranges must be sorted and disjoint"
        );

        let min_end = range_maps.iter().map(|rm| rm.end().unwrap_or(begin)).min().unwrap();

        let right_value_iters = range_maps.iter().map(|rm| rm.entries_after(begin).peekable()).collect::<Vec<_>>();

        // The edges of the excluded ranges split the segments, so that none crosses an edge.
        let edges: Box<dyn Iterator<Item = Bound> + 'a> =
            Box::new(excluded.iter().flat_map(|r| [r.start, r.end]).filter(move |&k| k > begin));
        let boundaries = range_maps.iter().map(move |rm| {
            let rights: Box<dyn Iterator<Item = Bound> + 'a> = Box::new(rm.entries_after(begin).map(|&(k, _)| k));
            rights
        });
        let sorted = boundaries.chain([edges]).kmerge().dedup();
        let merged_boundaries: Box<dyn Iterator<Item = Bound> + 'a> = Box::new(boundaries_through(sorted, min_end));

        Self {
            merged_boundaries,
            right_value_iters,
            left: begin,
            excluded,
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
        loop {
            let right = self.merged_boundaries.next()?;

            let values = std::array::from_fn(|i| {
                let &&(entry_right, val) =
                    self.right_value_iters[i].peek().expect("right_value_iter must have covering entry");
                if entry_right <= right {
                    self.right_value_iters[i].next();
                }
                val
            });

            let range = self.left..right;
            self.left = right;

            let excluded = self.excluded.iter().any(|r| r.start <= range.start && range.end <= r.end);
            if !excluded {
                return Some(RangeValues { range, values });
            }
        }
    }
}

/// Yields the boundaries of the sorted stream `sorted` up to and including `end`.
///
/// It stops at the first boundary beyond `end`: every later one is beyond it too, so the rest of
/// the stream, such as the long tail of a stage that extends past the shortest one, is not read.
fn boundaries_through<Bound>(sorted: impl Iterator<Item = Bound>, end: Bound) -> impl Iterator<Item = Bound>
where Bound: RangeMapBound {
    sorted.take_while(move |&k| k <= end)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::boundaries_through;

    #[test]
    fn test_boundaries_through_stops_at_the_first_boundary_beyond_the_end() {
        let read = Cell::new(0);
        let sorted = (1..=1000u64).inspect(|_| read.set(read.get() + 1));

        let boundaries: Vec<_> = boundaries_through(sorted, 3).collect();

        assert_eq!(vec![1, 2, 3], boundaries, "the end is included");
        assert_eq!(4, read.get(), "only the first boundary beyond the end is read");
    }
}
