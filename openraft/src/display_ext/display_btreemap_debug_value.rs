use std::collections::BTreeMap;
use std::fmt;

/// Implement `Display` for `BTreeMap<K, V>` if `K` is `Display` and `V` is `Debug`.
///
/// It formats a key-value pair in a string like "{key}:{value:?}" and
/// concatenates the key-value pairs with comma.
pub(crate) struct DisplayBTreeMapDebugValue<'a, K: fmt::Display, V: fmt::Debug>(pub &'a BTreeMap<K, V>);

impl<K: fmt::Display, V: fmt::Debug> fmt::Display for DisplayBTreeMapDebugValue<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.0.len();
        for (idx, (key, value)) in self.0.iter().enumerate() {
            write!(f, "{}:{:?}", key, value)?;
            if idx + 1 != len {
                write!(f, ",")?;
            }
        }

        Ok(())
    }
}

#[allow(unused)]
pub(crate) trait DisplayBTreeMapDebugValueExt<'a, K: fmt::Display, V: fmt::Debug> {
    fn display(&'a self) -> DisplayBTreeMapDebugValue<'a, K, V>;
}

impl<K, V> DisplayBTreeMapDebugValueExt<'_, K, V> for BTreeMap<K, V>
where
    K: fmt::Display,
    V: fmt::Debug,
{
    fn display(&self) -> DisplayBTreeMapDebugValue<'_, K, V> {
        DisplayBTreeMapDebugValue(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_btreemap_debug_value() {
        let map = (1..=3).map(|num| (num, num)).collect::<BTreeMap<_, _>>();
        let display = DisplayBTreeMapDebugValue(&map);

        assert_eq!(display.to_string(), "1:1,2:2,3:3");
    }

    #[test]
    fn test_display_empty_map() {
        let map = BTreeMap::<i32, i32>::new();
        let display = DisplayBTreeMapDebugValue(&map);

        assert_eq!(display.to_string(), "");
    }

    #[test]
    fn test_display_btreemap_debug_value_with_1_item() {
        let map = (1..=1).map(|num| (num, num)).collect::<BTreeMap<_, _>>();
        let display = DisplayBTreeMapDebugValue(&map);

        assert_eq!(display.to_string(), "1:1");
    }
}
