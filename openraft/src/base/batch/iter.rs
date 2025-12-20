//! Iterator implementation for `Batch`.

use std::vec;

use super::Batch;

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
    fn test_into_iter() {
        let v: Vec<_> = Batch::Single(42).into_iter().collect();
        assert_eq!(v, vec![42]);

        let v: Vec<_> = Batch::Vec(vec![1, 2, 3]).into_iter().collect();
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn test_exact_size() {
        assert_eq!(Batch::Single(42).into_iter().len(), 1);
        assert_eq!(Batch::Vec(vec![1, 2, 3]).into_iter().len(), 3);
    }
}
