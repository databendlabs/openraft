//! Update published metrics in place.
//!
//! `RaftCore` refreshes its metrics on every loop round, and a heartbeat changes some of them.
//! Updating each field in place, instead of building new metrics and cloning them into the
//! channels, costs no allocation while the set of nodes stays the same.

use std::collections::BTreeMap;

/// Set `slot` to `value` if they differ, and return whether `slot` changed.
pub(crate) fn set_if_ne<T>(slot: &mut T, value: T) -> bool
where T: PartialEq {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

/// Set `slot` to the map of the pairs that `entries` yields, or to `None` if there are no
/// entries, and return whether `slot` changed.
///
/// `entries` yields each key once. An existing map is updated in place while the keys stay the
/// same; when they differ, it is rebuilt.
pub(crate) fn set_map_if_ne<'a, K, V, E, I>(slot: &mut Option<BTreeMap<K, V>>, entries: Option<E>) -> bool
where
    K: Ord + Clone + 'a,
    V: PartialEq,
    E: Fn() -> I,
    I: Iterator<Item = (&'a K, V)>,
{
    let Some(entries) = entries else {
        return slot.take().is_some();
    };

    let collect = || entries().map(|(key, value)| (key.clone(), value)).collect();

    let map = match slot {
        Some(map) => map,
        None => {
            *slot = Some(collect());
            return true;
        }
    };

    let mut changed = false;
    let mut count = 0;
    for (key, value) in entries() {
        count += 1;
        match map.get_mut(key) {
            Some(current) => changed |= set_if_ne(current, value),
            None => {
                map.insert(key.clone(), value);
                changed = true;
            }
        }
    }

    // Every key that `entries` yields is in the map now, so a map with more keys holds one that
    // `entries` no longer yields.
    if map.len() != count {
        *map = collect();
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::set_if_ne;
    use super::set_map_if_ne;

    #[test]
    fn test_set_if_ne_reports_whether_the_value_changed() {
        let mut slot = 1;
        assert!(!set_if_ne(&mut slot, 1));
        assert!(set_if_ne(&mut slot, 2));
        assert_eq!(2, slot);
    }

    #[test]
    fn test_set_map_if_ne_follows_the_entries() {
        let ids: &[u64] = &[1, 2, 3];
        let none = None::<fn() -> std::iter::Empty<(&'static u64, u8)>>;
        let entries = |values: [u8; 3]| Some(move || ids.iter().zip(values));
        let map = |pairs: &[(u64, u8)]| Some(pairs.iter().copied().collect::<BTreeMap<_, _>>());

        tracing::info!("no entries clear the map, and a first map is built");
        let mut slot = map(&[(9, 9)]);
        {
            assert!(set_map_if_ne(&mut slot, none));
            assert_eq!(None, slot);
            assert!(!set_map_if_ne(&mut slot, none));

            assert!(set_map_if_ne(&mut slot, entries([1, 1, 1])));
            assert_eq!(map(&[(1, 1), (2, 1), (3, 1)]), slot);
        }

        tracing::info!("the same keys update the map in place");
        {
            assert!(!set_map_if_ne(&mut slot, entries([1, 1, 1])));
            assert!(set_map_if_ne(&mut slot, entries([1, 5, 1])));
            assert_eq!(map(&[(1, 1), (2, 5), (3, 1)]), slot);
        }

        tracing::info!("a changed key set rebuilds the map");
        {
            let fewer = Some(|| ids[..2].iter().zip([1u8, 5]));
            assert!(set_map_if_ne(&mut slot, fewer));
            assert_eq!(map(&[(1, 1), (2, 5)]), slot);

            let other: &[u64] = &[1, 4];
            let replaced = Some(|| other.iter().zip([1u8, 7]));
            assert!(set_map_if_ne(&mut slot, replaced));
            assert_eq!(map(&[(1, 1), (4, 7)]), slot);
        }
    }
}
