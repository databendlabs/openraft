use std::collections::VecDeque;
use std::ops::Deref;
use std::ops::DerefMut;

use super::slot::Slot;

/// A slot container with stable logical capacity semantics.
///
/// The logical slot limit is tracked separately from the underlying `VecDeque`
/// allocation capacity, which may grow beyond the requested value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotQueue<T> {
    capacity: usize,
    slots: VecDeque<Slot<T>>,
}

impl<T> SlotQueue<T> {
    pub(crate) fn new(capacity: usize, num_buckets: usize) -> Self {
        let mut slots = VecDeque::with_capacity(capacity);
        slots.push_back(Slot::new(num_buckets));

        Self { capacity, slots }
    }

    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<T> Deref for SlotQueue<T> {
    type Target = VecDeque<Slot<T>>;

    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl<T> DerefMut for SlotQueue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slots
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use super::*;

    #[test]
    fn test_capacity_is_stable_when_vecdeque_reserves_more() {
        let mut slots: SlotQueue<()> = SlotQueue::new(3, 4);

        assert_eq!(slots.capacity(), 3);

        slots.reserve(16);

        assert!(Deref::deref(&slots).capacity() > slots.capacity());
        assert_eq!(slots.capacity(), 3);
    }
}
