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
