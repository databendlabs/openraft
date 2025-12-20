//! A container that stores elements efficiently by avoiding heap allocation for single elements.

mod display;
mod iter;

use std::ops::Index;
use std::slice;

/// A container that stores elements efficiently by avoiding heap allocation for single elements.
///
/// This type uses an enum with two variants:
/// - `Single`: stores exactly one element inline (no heap allocation)
/// - `Vec`: stores zero or more elements using a `Vec`
#[derive(Debug, Clone, Eq)]
pub(crate) enum Batch<T> {
    /// A single element stored inline without heap allocation.
    Single(T),
    /// Multiple elements stored in a Vec.
    Vec(Vec<T>),
}

/// `PartialEq` compares by content, making `Single(x) == Vec([x])`.
impl<T: PartialEq> PartialEq for Batch<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T> Default for Batch<T> {
    fn default() -> Self {
        Batch::Vec(Vec::new())
    }
}

#[allow(dead_code)]
impl<T> Batch<T> {
    /// Returns the number of elements.
    pub fn len(&self) -> usize {
        match self {
            Batch::Single(_) => 1,
            Batch::Vec(v) => v.len(),
        }
    }

    /// Returns true if there are no elements.
    pub fn is_empty(&self) -> bool {
        match self {
            Batch::Single(_) => false,
            Batch::Vec(v) => v.is_empty(),
        }
    }

    /// Returns a slice of all elements.
    pub fn as_slice(&self) -> &[T] {
        match self {
            Batch::Single(e) => slice::from_ref(e),
            Batch::Vec(v) => v.as_slice(),
        }
    }

    /// Returns a mutable slice of all elements.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        match self {
            Batch::Single(e) => slice::from_mut(e),
            Batch::Vec(v) => v.as_mut_slice(),
        }
    }

    /// Returns the first element, or None if empty.
    pub fn first(&self) -> Option<&T> {
        match self {
            Batch::Single(e) => Some(e),
            Batch::Vec(v) => v.first(),
        }
    }

    /// Returns the last element, or None if empty.
    pub fn last(&self) -> Option<&T> {
        match self {
            Batch::Single(e) => Some(e),
            Batch::Vec(v) => v.last(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_conversions() {
        // From single value
        assert_eq!(Batch::from(42), Batch::Single(42));

        // From single-element vec/array becomes Single
        assert_eq!(Batch::from(vec![42]), Batch::Single(42));
        assert_eq!(Batch::<i32>::from([42]), Batch::Single(42));

        // From multi-element vec/array becomes Vec
        assert_eq!(Batch::from(vec![1, 2, 3]), Batch::Vec(vec![1, 2, 3]));
        assert_eq!(Batch::<i32>::from([1, 2, 3]), Batch::Vec(vec![1, 2, 3]));

        // From empty vec/array becomes empty Vec
        assert_eq!(Batch::<i32>::from(vec![]), Batch::Vec(vec![]));
        assert_eq!(Batch::<i32>::from([]), Batch::Vec(vec![]));
    }

    #[test]
    fn test_extend() {
        let mut v: Batch<i32> = 1.into();
        v.extend(2.into());
        assert_eq!(v, Batch::Vec(vec![1, 2]));

        let mut v: Batch<i32> = 1.into();
        v.extend([2, 3].into());
        assert_eq!(v, Batch::Vec(vec![1, 2, 3]));

        let mut v: Batch<i32> = [1, 2].into();
        v.extend(3.into());
        assert_eq!(v, Batch::Vec(vec![1, 2, 3]));

        let mut v: Batch<i32> = [1, 2].into();
        v.extend([3, 4].into());
        assert_eq!(v, Batch::Vec(vec![1, 2, 3, 4]));
    }

    #[test]
    fn test_equality_across_variants() {
        // Single and Vec with same content are equal
        assert_eq!(Batch::Single(42), Batch::Vec(vec![42]));
        assert_ne!(Batch::Single(1), Batch::Vec(vec![1, 2]));
    }

    #[test]
    fn test_clone() {
        assert_eq!(Batch::Single(42).clone(), Batch::Single(42));
        assert_eq!(Batch::Vec(vec![1, 2]).clone(), Batch::Vec(vec![1, 2]));
    }

    #[test]
    fn test_default() {
        assert_eq!(Batch::<i32>::default(), Batch::Vec(vec![]));
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn test_index_out_of_bounds() {
        let _ = Batch::Single(42)[1];
    }

    #[test]
    fn test_debug() {
        assert_eq!(format!("{:?}", Batch::Single(42)), "Single(42)");
        assert_eq!(format!("{:?}", Batch::Vec(vec![1, 2])), "Vec([1, 2])");
    }
}
