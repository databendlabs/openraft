/// A single slot containing bucket counts and user-defined metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot<T> {
    /// Count of samples in each bucket.
    pub(crate) buckets: Vec<u64>,
    /// User-defined metadata for this slot. Only set when slot is activated via `advance()`.
    pub(crate) data: Option<T>,
}

impl<T> Slot<T> {
    pub(crate) fn new(num_buckets: usize) -> Self {
        Self {
            buckets: vec![0; num_buckets],
            data: None,
        }
    }

    /// Clears the slot: resets all bucket counts to zero and removes user data.
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.buckets.fill(0);
        self.data = None;
    }
}
