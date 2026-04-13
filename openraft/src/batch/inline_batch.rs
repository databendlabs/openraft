use smallvec::SmallVec;

/// A container that stores elements efficiently by avoiding heap allocation for single elements.
///
/// Internally backed by `SmallVec<[T; 1]>`, which stores a single element inline
/// and falls back to heap allocation for multiple elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineBatch<T> {
    inner: SmallVec<[T; 1]>,
}

impl<T> AsRef<[T]> for InlineBatch<T> {
    fn as_ref(&self) -> &[T] {
        self.inner.as_ref()
    }
}

impl<T> Extend<T> for InlineBatch<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}

impl<T> IntoIterator for InlineBatch<T> {
    type Item = T;
    type IntoIter = smallvec::IntoIter<[T; 1]>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<T> FromIterator<T> for InlineBatch<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        InlineBatch {
            inner: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch<T>(items: impl IntoIterator<Item = T>) -> InlineBatch<T> {
        items.into_iter().collect()
    }

    #[test]
    fn test_collect() {
        assert_eq!(batch([42]).as_ref(), &[42]);
        assert_eq!(batch([1, 2, 3]).as_ref(), &[1, 2, 3]);
        assert_eq!(batch::<i32>([]).as_ref(), &[] as &[i32]);
    }

    #[test]
    fn test_extend() {
        let mut v = batch([1]);
        v.extend([2]);
        assert_eq!(v.as_ref(), &[1, 2]);

        let mut v = batch([1]);
        v.extend([2, 3]);
        assert_eq!(v.as_ref(), &[1, 2, 3]);

        let mut v = batch([1, 2]);
        v.extend([3]);
        assert_eq!(v.as_ref(), &[1, 2, 3]);

        let mut v = batch([1, 2]);
        v.extend([3, 4]);
        assert_eq!(v.as_ref(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_clone() {
        assert_eq!(batch([42]).clone(), batch([42]));
        assert_eq!(batch([1, 2]).clone(), batch([1, 2]));
    }

    #[test]
    fn test_empty() {
        assert_eq!(batch::<i32>([]).as_ref(), &[] as &[i32]);
    }
}
