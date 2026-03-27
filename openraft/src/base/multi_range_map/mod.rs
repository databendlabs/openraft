mod segment_iter;
#[cfg(test)]
mod tests;

pub use segment_iter::SegmentIter;

use super::range_map::RangeMap;
use super::range_map::RangeMapBound;
use super::range_map::RangeMapValue;

/// A collection of `N` [`RangeMap`]s sharing the same key and value types.
///
/// Each range map tracks a separate channel/stage. The [`segments()`](Self::segments)
/// method K-way merges boundaries from all ranges and yields
/// [`RangeValues`](super::range_values::RangeValues) segments where no range map's
/// batch boundary is crossed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiRangeMap<Bound, V, const N: usize>
where
    Bound: RangeMapBound,
    V: RangeMapValue,
{
    range_maps: Vec<RangeMap<Bound, V>>,
}

impl<Bound, V, const N: usize> MultiRangeMap<Bound, V, N>
where
    Bound: RangeMapBound,
    V: RangeMapValue,
{
    pub fn new(capacity: usize) -> Self {
        Self {
            range_maps: (0..N).map(|_| RangeMap::new(capacity)).collect(),
        }
    }

    pub fn get_mut(&mut self, index: usize) -> &mut RangeMap<Bound, V> {
        &mut self.range_maps[index]
    }

    /// Iterate segments: contiguous key ranges that do not cross any range map's
    /// batch boundary, within the intersection range where all ranges have data.
    ///
    /// `begin` is the left boundary of the first segment.
    #[allow(dead_code)]
    pub fn segments(&self, begin: Bound) -> SegmentIter<'_, Bound, V, N> {
        SegmentIter::new(begin, &self.range_maps)
    }
}
