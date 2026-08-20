/// Orders pending reads by quorum-acknowledgement threshold and insertion sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PendingReadKey<I> {
    pub(super) min_quorum_acked_at: I,
    pub(super) sequence: u64,
}
