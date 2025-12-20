//! A container that stores elements efficiently by avoiding heap allocation for single elements.

use std::fmt;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::ops::Index;
use std::slice;
use std::vec;

/// A container that stores elements efficiently by avoiding heap allocation for single elements.
///
/// This type uses an enum with two variants:
/// - `Single`: stores exactly one element inline (no heap allocation)
/// - `Vec`: stores zero or more elements using a `Vec`
pub enum Batch<T> {
    /// A single element stored inline without heap allocation.
    Single(T),
    /// Multiple elements stored in a Vec.
    Vec(Vec<T>),
}

impl<T: Debug> Debug for Batch<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Batch::Single(e) => f.debug_tuple("Single").field(e).finish(),
            Batch::Vec(v) => f.debug_tuple("Vec").field(v).finish(),
        }
    }
}

impl<T: Clone> Clone for Batch<T> {
    fn clone(&self) -> Self {
        match self {
            Batch::Single(e) => Batch::Single(e.clone()),
            Batch::Vec(v) => Batch::Vec(v.clone()),
        }
    }
}

impl<T: PartialEq> PartialEq for Batch<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Eq> Eq for Batch<T> {}

impl<T> Default for Batch<T> {
    fn default() -> Self {
        Batch::Vec(Vec::new())
    }
}

impl<T> Batch<T> {
    /// Returns the number of elements.
    pub fn len(&self) -> usize {
        match self {
            Batch::Single(_) => 1,
            Batch::Vec(v) => v.len(),
        }
    }

    /// Returns a slice of all elements.
    pub fn as_slice(&self) -> &[T] {
        match self {
            Batch::Single(e) => slice::from_ref(e),
            Batch::Vec(v) => v.as_slice(),
        }
    }

    /// Returns the last element, or None if empty.
    pub fn last(&self) -> Option<&T> {
        match self {
            Batch::Single(e) => Some(e),
            Batch::Vec(v) => v.last(),
        }
    }

    /// Appends elements from another `Batch`.
    ///
    /// This method converts `self` to the `Vec` variant if needed.
    pub fn extend(&mut self, other: Batch<T>) {
        match self {
            Batch::Single(_) => {
                // Convert single to vec, then extend
                let single = std::mem::replace(self, Batch::Vec(Vec::new()));
                let Batch::Single(e) = single else { unreachable!() };
                let Batch::Vec(v) = self else { unreachable!() };
                v.push(e);
                match other {
                    Batch::Single(o) => v.push(o),
                    Batch::Vec(ov) => v.extend(ov),
                }
            }
            Batch::Vec(v) => match other {
                Batch::Single(e) => v.push(e),
                Batch::Vec(ov) => v.extend(ov),
            },
        }
    }
}

#[cfg(test)]
impl<T> Batch<T> {
    /// Returns the first element, or None if empty.
    pub fn first(&self) -> Option<&T> {
        match self {
            Batch::Single(e) => Some(e),
            Batch::Vec(v) => v.first(),
        }
    }

    /// Returns true if there are no elements.
    pub fn is_empty(&self) -> bool {
        match self {
            Batch::Single(_) => false,
            Batch::Vec(v) => v.is_empty(),
        }
    }

    /// Returns a mutable slice of all elements.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        match self {
            Batch::Single(e) => slice::from_mut(e),
            Batch::Vec(v) => v.as_mut_slice(),
        }
    }

    /// Returns an iterator over the elements.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &T> {
        self.as_slice().iter()
    }

    /// Returns a mutable iterator over the elements.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.as_mut_slice().iter_mut()
    }
}

impl<T: fmt::Display> Batch<T> {
    /// Returns a display helper that shows all elements.
    pub fn display(&self) -> impl fmt::Display + '_ {
        BatchDisplay {
            elements: self,
            max: None,
        }
    }

    /// Returns a display helper that shows at most `max` elements.
    pub fn display_n(&self, max: usize) -> impl fmt::Display + '_ {
        BatchDisplay {
            elements: self,
            max: Some(max),
        }
    }
}

struct BatchDisplay<'a, T> {
    elements: &'a Batch<T>,
    max: Option<usize>,
}

impl<'a, T: fmt::Display> fmt::Display for BatchDisplay<'a, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let slice = self.elements.as_slice();
        let max = self.max.unwrap_or(slice.len());
        let len = slice.len();
        let shown = max.min(len);

        write!(f, "[")?;
        for (i, e) in slice.iter().take(max).enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", e)?;
        }
        if len > max {
            if shown > 0 {
                write!(f, ", ")?;
            }
            write!(f, "... {} more", len - max)?;
        }
        write!(f, "]")
    }
}

impl<T> Index<usize> for Batch<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        match self {
            Batch::Single(e) => {
                assert!(
                    index == 0,
                    "index out of bounds: the len is 1 but the index is {}",
                    index
                );
                e
            }
            Batch::Vec(v) => &v[index],
        }
    }
}

impl<T> From<T> for Batch<T> {
    fn from(element: T) -> Self {
        Batch::Single(element)
    }
}

impl<T> From<Vec<T>> for Batch<T> {
    fn from(mut elements: Vec<T>) -> Self {
        if elements.len() == 1 {
            Batch::Single(elements.pop().unwrap())
        } else {
            Batch::Vec(elements)
        }
    }
}

impl<T, const N: usize> From<[T; N]> for Batch<T> {
    fn from(arr: [T; N]) -> Self {
        if N == 1 {
            let mut iter = arr.into_iter();
            Batch::Single(iter.next().unwrap())
        } else {
            Batch::Vec(Vec::from(arr))
        }
    }
}

impl<T> IntoIterator for Batch<T> {
    type Item = T;
    type IntoIter = BatchIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Batch::Single(e) => BatchIter::Single(Some(e)),
            Batch::Vec(v) => BatchIter::Vec(v.into_iter()),
        }
    }
}

/// An iterator over the elements in a `Batch`.
pub enum BatchIter<T> {
    Single(Option<T>),
    Vec(vec::IntoIter<T>),
}

impl<T> Iterator for BatchIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            BatchIter::Single(e) => e.take(),
            BatchIter::Vec(v) => v.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match self {
            BatchIter::Single(Some(_)) => 1,
            BatchIter::Single(None) => 0,
            BatchIter::Vec(v) => v.len(),
        };
        (len, Some(len))
    }
}

impl<T> ExactSizeIterator for BatchIter<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_basic() {
        let v: Batch<i32> = 42.into();

        assert_eq!(v.len(), 1);
        assert!(!v.is_empty());
        assert_eq!(v.first(), Some(&42));
        assert_eq!(v.last(), Some(&42));
        assert_eq!(v.as_slice(), &[42]);
        assert_eq!(v[0], 42);
    }

    #[test]
    fn test_vec_basic() {
        let v: Batch<i32> = [1, 2, 3].into();

        assert_eq!(v.len(), 3);
        assert!(!v.is_empty());
        assert_eq!(v.first(), Some(&1));
        assert_eq!(v.last(), Some(&3));
        assert_eq!(v.as_slice(), &[1, 2, 3]);
        assert_eq!(v[0], 1);
        assert_eq!(v[1], 2);
        assert_eq!(v[2], 3);
    }

    #[test]
    fn test_empty_vec() {
        let v: Batch<i32> = [].into();

        assert_eq!(v.len(), 0);
        assert!(v.is_empty());
        assert_eq!(v.first(), None);
        assert_eq!(v.last(), None);
        let expected: &[i32] = &[];
        assert_eq!(v.as_slice(), expected);
    }

    #[test]
    fn test_from_single() {
        let v: Batch<i32> = Batch::from(42);
        assert_eq!(v.as_slice(), &[42]);
    }

    #[test]
    fn test_from_vec() {
        let v: Batch<i32> = Batch::from(vec![1, 2, 3]);
        assert_eq!(v.as_slice(), &[1, 2, 3]);
        assert!(matches!(v, Batch::Vec(_)));

        // Single-element Vec becomes Single variant
        let v2: Batch<i32> = Batch::from(vec![42]);
        assert_eq!(v2.as_slice(), &[42]);
        assert!(matches!(v2, Batch::Single(_)));

        // Empty Vec stays as Vec variant
        let v3: Batch<i32> = Batch::from(vec![]);
        assert!(v3.is_empty());
        assert!(matches!(v3, Batch::Vec(_)));
    }

    #[test]
    fn test_from_array() {
        let v: Batch<i32> = [1, 2, 3].into();
        assert_eq!(v.as_slice(), &[1, 2, 3]);
        assert!(matches!(v, Batch::Vec(_)));

        // Single-element array becomes Single variant
        let v2: Batch<i32> = [42].into();
        assert_eq!(v2.as_slice(), &[42]);
        assert!(matches!(v2, Batch::Single(_)));

        // Empty array becomes empty Vec variant
        let v3: Batch<i32> = [].into();
        assert!(v3.is_empty());
        assert!(matches!(v3, Batch::Vec(_)));
    }

    #[test]
    fn test_iter_single() {
        let v: Batch<i32> = 42.into();
        let collected: Vec<_> = v.iter().cloned().collect();
        assert_eq!(collected, vec![42]);
    }

    #[test]
    fn test_iter_vec() {
        let v: Batch<i32> = [1, 2, 3].into();
        let collected: Vec<_> = v.iter().cloned().collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }

    #[test]
    fn test_into_iter_single() {
        let v: Batch<i32> = 42.into();
        let collected: Vec<_> = v.into_iter().collect();
        assert_eq!(collected, vec![42]);
    }

    #[test]
    fn test_into_iter_vec() {
        let v: Batch<i32> = [1, 2, 3].into();
        let collected: Vec<_> = v.into_iter().collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }

    #[test]
    fn test_extend_single_with_single() {
        let mut v: Batch<i32> = 1.into();
        v.extend(2.into());
        assert_eq!(v.as_slice(), &[1, 2]);
    }

    #[test]
    fn test_extend_single_with_vec() {
        let mut v: Batch<i32> = 1.into();
        v.extend([2, 3].into());
        assert_eq!(v.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn test_extend_vec_with_single() {
        let mut v: Batch<i32> = [1, 2].into();
        v.extend(3.into());
        assert_eq!(v.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn test_extend_vec_with_vec() {
        let mut v: Batch<i32> = [1, 2].into();
        v.extend([3, 4].into());
        assert_eq!(v.as_slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_equality() {
        let v1: Batch<i32> = 42.into();
        let v2: Batch<i32> = [42].into();
        assert_eq!(v1, v2);

        let v3: Batch<i32> = [1, 2].into();
        let v4: Batch<i32> = [1, 2].into();
        assert_eq!(v3, v4);

        let v5: Batch<i32> = 1.into();
        let v6: Batch<i32> = [1, 2].into();
        assert_ne!(v5, v6);
    }

    #[test]
    fn test_clone() {
        let v1: Batch<i32> = 42.into();
        let v2 = v1.clone();
        assert_eq!(v1, v2);

        let v3: Batch<i32> = [1, 2, 3].into();
        let v4 = v3.clone();
        assert_eq!(v3, v4);
    }

    #[test]
    fn test_default() {
        let v: Batch<i32> = Batch::default();
        assert!(v.is_empty());
        assert_eq!(v.len(), 0);
    }

    #[test]
    fn test_iter_exact_size() {
        let v: Batch<i32> = [1, 2, 3].into();
        let iter = v.iter();
        assert_eq!(iter.len(), 3);

        let v2: Batch<i32> = 42.into();
        let iter2 = v2.iter();
        assert_eq!(iter2.len(), 1);
    }

    #[test]
    fn test_into_iter_exact_size() {
        let v: Batch<i32> = [1, 2, 3].into();
        let iter = v.into_iter();
        assert_eq!(iter.len(), 3);

        let v2: Batch<i32> = 42.into();
        let iter2 = v2.into_iter();
        assert_eq!(iter2.len(), 1);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn test_index_out_of_bounds_single() {
        let v: Batch<i32> = 42.into();
        let _ = v[1];
    }

    #[test]
    fn test_iter_mut() {
        let mut v: Batch<i32> = [1, 2, 3].into();
        for x in v.iter_mut() {
            *x *= 2;
        }
        assert_eq!(v.as_slice(), &[2, 4, 6]);

        let mut v2: Batch<i32> = 5.into();
        for x in v2.iter_mut() {
            *x *= 3;
        }
        assert_eq!(v2.as_slice(), &[15]);
    }

    #[test]
    fn test_debug() {
        let v1: Batch<i32> = 42.into();
        assert_eq!(format!("{:?}", v1), "Single(42)");

        let v2: Batch<i32> = [1, 2, 3].into();
        assert_eq!(format!("{:?}", v2), "Vec([1, 2, 3])");

        let v3: Batch<i32> = [].into();
        assert_eq!(format!("{:?}", v3), "Vec([])");
    }

    #[test]
    fn test_display() {
        let v1: Batch<i32> = 42.into();
        assert_eq!(format!("{}", v1.display()), "[42]");

        let v2: Batch<i32> = [1, 2, 3].into();
        assert_eq!(format!("{}", v2.display()), "[1, 2, 3]");

        let v3: Batch<i32> = [].into();
        assert_eq!(format!("{}", v3.display()), "[]");
    }

    #[test]
    fn test_display_n() {
        let v: Batch<i32> = [1, 2, 3, 4, 5].into();

        assert_eq!(format!("{}", v.display_n(3)), "[1, 2, 3, ... 2 more]");
        assert_eq!(format!("{}", v.display_n(5)), "[1, 2, 3, 4, 5]");
        assert_eq!(format!("{}", v.display_n(10)), "[1, 2, 3, 4, 5]");
        assert_eq!(format!("{}", v.display_n(0)), "[... 5 more]");

        let v2: Batch<i32> = 42.into();
        assert_eq!(format!("{}", v2.display_n(0)), "[... 1 more]");
        assert_eq!(format!("{}", v2.display_n(1)), "[42]");
    }
}
