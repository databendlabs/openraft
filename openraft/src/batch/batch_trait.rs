use crate::OptionalSend;

/// Trait defining the interface for a batch container.
///
/// A `Batch` is a collection that can be created from an iterator,
/// extended with new elements, iterated over, and viewed as a slice.
/// The default implementation is [`InlineBatch`](`super::InlineBatch`),
/// backed by `SmallVec<[T; 1]>`.
pub trait Batch<T>:
    AsRef<[T]> + Extend<T> + IntoIterator<Item = T, IntoIter: OptionalSend> + FromIterator<T> + OptionalSend
{
    /// Creates a `Batch` from any iterable.
    fn of(items: impl IntoIterator<Item = T>) -> Self {
        items.into_iter().collect()
    }

    /// Returns the number of elements.
    fn len(&self) -> usize {
        self.as_ref().len()
    }

    /// Returns `true` if the batch contains no elements.
    fn is_empty(&self) -> bool {
        self.as_ref().is_empty()
    }

    /// Returns a reference to the last element, or `None` if empty.
    fn last(&self) -> Option<&T> {
        self.as_ref().last()
    }
}

impl<T, B> Batch<T> for B where B: AsRef<[T]> + Extend<T> + IntoIterator<Item = T, IntoIter: OptionalSend> + FromIterator<T> + OptionalSend
{}
