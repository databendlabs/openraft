use std::ops::Range;

use crate::base::range_map::RangeMapBound;
use crate::base::range_map::RangeMapValue;

/// A segment defined by a key range with a fixed number of associated values.
///
/// Represents the range `[start, end)` with `N` values, one per channel/stage.
#[allow(dead_code)]
pub struct RangeValues<Bound, V, const N: usize>
where
    Bound: RangeMapBound,
    V: RangeMapValue,
{
    pub range: Range<Bound>,
    pub values: [V; N],
}
