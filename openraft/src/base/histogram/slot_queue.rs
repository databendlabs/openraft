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
    slot_limit: usize,
    slots: VecDeque<Slot<T>>,
}

impl<T> SlotQueue<T> {
    pub(crate) fn new(slot_limit: usize, num_buckets: usize) -> Self {
        let mut slots = VecDeque::with_capacity(slot_limit);
        slots.push_back(Slot::new(num_buckets));

        Self { slot_limit, slots }
    }

    #[inline]
    pub(crate) fn slot_limit(&self) -> usize {
        self.slot_limit
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
    fn test_slot_limit_is_stable_when_vecdeque_reserves_more() {
        let mut slots: SlotQueue<()> = SlotQueue::new(3, 4);

        assert_eq!(slots.slot_limit(), 3);

        slots.reserve(16);

        assert!(Deref::deref(&slots).capacity() > slots.slot_limit());
        assert_eq!(slots.slot_limit(), 3);
    }
}
