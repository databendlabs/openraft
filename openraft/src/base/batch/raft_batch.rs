//! Trait definition for customizable batch containers in Raft.

use std::fmt::Debug;
use std::fmt::Display;

use super::display::DisplayBatch;
use crate::OptionalSend;

/// Trait for batch containers used throughout Raft for efficient element grouping.
///
/// This trait abstracts over different batch container implementations while
/// preserving performance characteristics required by Raft.
///
/// Implementations may optimize for:
/// - Avoiding heap allocation for small batches
/// - Arena or pool-backed allocation
/// - Cache locality
///
/// # Design Notes
///
/// - The trait is `Sized` to enable static dispatch and full monomorphization.
/// - Iteration is explicit via `iter()` / `iter_mut()` to preserve lifetime and `ExactSizeIterator`
///   guarantees.
/// - Consuming iteration via `into_iter()` method with explicit `IntoIter` associated type.
pub trait RaftBatch<T>: OptionalSend + Sized + Default + Debug + 'static
where T: OptionalSend + Debug + 'static
{
    /// Iterator type for immutable element references.
    ///
    /// Must implement `ExactSizeIterator` to allow efficient size queries.
    type Iter<'a>: Iterator<Item = &'a T> + ExactSizeIterator
    where
        T: 'a,
        Self: 'a;

    /// Iterator type for mutable element references.
    ///
    /// Must implement `ExactSizeIterator` to allow efficient size queries.
    type IterMut<'a>: Iterator<Item = &'a mut T> + ExactSizeIterator
    where
        T: 'a,
        Self: 'a;

    /// Iterator type for consuming iteration.
    ///
    /// Must implement `ExactSizeIterator` to allow efficient size queries.
    type IntoIter: Iterator<Item = T> + ExactSizeIterator + OptionalSend;

    /// Creates a batch containing a single item.
    ///
    /// Implementations may optimize this case to avoid heap allocation.
    fn from_item(item: T) -> Self;

    /// Creates a batch from a `Vec`.
    ///
    /// Implementations may optimize for the single-element case.
    fn from_vec(vec: Vec<T>) -> Self;

    /// Creates a batch from an exact-size iterator.
    ///
    /// The exact size allows implementations to pre-allocate or select
    /// efficient internal representations.
    fn from_exact_iter<I>(iter: I) -> Self
    where I: ExactSizeIterator<Item = T>;

    /// Returns the number of elements in the batch.
    fn len(&self) -> usize;

    /// Returns `true` if the batch contains no elements.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a reference to the first element, or `None` if empty.
    fn first(&self) -> Option<&T>;

    /// Returns a reference to the last element, or `None` if empty.
    fn last(&self) -> Option<&T>;

    /// Returns an iterator over immutable element references.
    fn iter(&self) -> Self::Iter<'_>;

    /// Returns an iterator over mutable element references.
    fn iter_mut(&mut self) -> Self::IterMut<'_>;

    /// Consumes the batch and returns an iterator over the elements.
    fn into_iter(self) -> Self::IntoIter;

    /// Appends all elements from another batch to this batch.
    ///
    /// This method consumes `other` to allow efficient transfer of ownership.
    fn extend(&mut self, other: Self);

    /// Returns a display helper that formats all elements.
    fn display(&self) -> DisplayBatch<'_, T, Self>
    where T: Display {
        DisplayBatch::new(self, None)
    }

    /// Returns a display helper that formats at most `max` elements.
    fn display_n(&self, max: usize) -> DisplayBatch<'_, T, Self>
    where T: Display {
        DisplayBatch::new(self, Some(max))
    }
}
