use std::collections::BTreeMap;
use std::fmt;

use super::DisplayOption;

/// Implement `Display` for `BTreeMap<K, Option<V>>` if `K` and `V` are `Display`.
///
/// It formats a key-value pair in a string like "{key}:{opt_value}" and
/// concatenates the key-value pairs with comma.
///
/// For how to format the `opt_value`, see [`DisplayOption`].
pub(crate) struct DisplayBTreeMapOptValue<'a, K: fmt::Display, V: fmt::Display>(pub &'a BTreeMap<K, Option<V>>);

impl<K: fmt::Display, V: fmt::Display> fmt::Display for DisplayBTreeMapOptValue<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.0.len();
        for (idx, (key, value)) in self.0.iter().enumerate() {
            write!(f, "{}:{}", key, DisplayOption(value))?;
            if idx + 1 != len {
                write!(f, ",")?;
            }
        }

        Ok(())
    }
}

#[allow(unused)]
pub(crate) trait DisplayBtreeMapOptValueExt<'a, K: fmt::Display, V: fmt::Display> {
    fn display(&'a self) -> DisplayBTreeMapOptValue<'a, K, V>;
}

impl<K, V> DisplayBtreeMapOptValueExt<'_, K, V> for BTreeMap<K, Option<V>>
where
    K: fmt::Display,
    V: fmt::Display,
{
    fn display(&self) -> DisplayBTreeMapOptValue<'_, K, V> {
        DisplayBTreeMapOptValue(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_btreemap_opt_value() {
        let map = (1..=3).map(|num| (num, Some(num))).collect::<BTreeMap<_, _>>();
        let display = DisplayBTreeMapOptValue(&map);

        assert_eq!(display.to_string(), "1:1,2:2,3:3");
    }

    #[test]
    fn test_display_empty_map() {
        let map = BTreeMap::<i32, Option<i32>>::new();
        let display = DisplayBTreeMapOptValue(&map);

        assert_eq!(display.to_string(), "");
    }

    #[test]
    fn test_display_btreemap_opt_value_with_1_item() {
        let map = (1..=1).map(|num| (num, Some(num))).collect::<BTreeMap<_, _>>();
        let display = DisplayBTreeMapOptValue(&map);

        assert_eq!(display.to_string(), "1:1");
    }

    #[test]
    fn test_display_btreemap_opt_value_with_none() {
        let mut map = BTreeMap::new();
        map.insert(1, Some(1));
        map.insert(2, None);
        map.insert(3, Some(3));
        let display = DisplayBTreeMapOptValue(&map);

        assert_eq!(display.to_string(), "1:1,2:None,3:3");
    }
}
