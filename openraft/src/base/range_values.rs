use std::ops::Range;

use crate::base::range_map::RangeMapBound;
use crate::base::range_map::RangeMapValue;

/// A segment defined by a key range with a fixed number of associated values.
///
/// Represents the range `[start, end)` with `N` values, one per channel/stage.
pub struct RangeValues<Bound, V, const N: usize>
where
    Bound: RangeMapBound,
    V: RangeMapValue,
{
    /// The key range `[start, end)`.
    pub range: Range<Bound>,
    /// One value per channel or stage.
    pub values: [V; N],
}
