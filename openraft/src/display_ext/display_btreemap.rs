use std::collections::BTreeMap;
use std::fmt;

/// Implement `Display` for `BTreeMap<K, V>` if `K` and `V` are `Display`.
///
/// It formats a key-value pair in a string like "{key}:{value}" and
/// concatenates the key-value pairs with comma.
pub(crate) struct DisplayBTreeMap<'a, K: fmt::Display, V: fmt::Display>(pub &'a BTreeMap<K, V>);

impl<K: fmt::Display, V: fmt::Display> fmt::Display for DisplayBTreeMap<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.0.len();
        for (idx, (key, value)) in self.0.iter().enumerate() {
            write!(f, "{}:{}", key, value)?;
            if idx + 1 != len {
                write!(f, ",")?;
            }
        }

        Ok(())
    }
}

#[allow(unused)]
pub(crate) trait DisplayBTreeMapExt<'a, K: fmt::Display, V: fmt::Display> {
    fn display(&'a self) -> DisplayBTreeMap<'a, K, V>;
}

impl<K, V> DisplayBTreeMapExt<'_, K, V> for BTreeMap<K, V>
where
    K: fmt::Display,
    V: fmt::Display,
{
    fn display(&self) -> DisplayBTreeMap<'_, K, V> {
        DisplayBTreeMap(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_btreemap() {
        let map = (1..=3).map(|num| (num, num)).collect::<BTreeMap<_, _>>();
        let display = DisplayBTreeMap(&map);

        assert_eq!(display.to_string(), "1:1,2:2,3:3");
    }

    #[test]
    fn test_display_empty_map() {
        let map = BTreeMap::<i32, i32>::new();
        let display = DisplayBTreeMap(&map);

        assert_eq!(display.to_string(), "");
    }

    #[test]
    fn test_display_btreemap_with_1_item() {
        let map = (1..=1).map(|num| (num, num)).collect::<BTreeMap<_, _>>();
        let display = DisplayBTreeMap(&map);

        assert_eq!(display.to_string(), "1:1");
    }
}
