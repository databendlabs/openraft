use std::ops::Range;

use crate::base::range_map::RangeMapKey;
use crate::base::range_map::RangeMapValue;

/// A segment defined by a key range with a fixed number of associated values.
///
/// Represents the range `[start, end)` with `N` values, one per channel/stage.
#[allow(dead_code)]
pub struct RangeValues<K, V, const N: usize>
where
    K: RangeMapKey,
    V: RangeMapValue,
{
    pub range: Range<K>,
    pub values: [V; N],
}
