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
