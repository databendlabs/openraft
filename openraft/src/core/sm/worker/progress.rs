/// Cumulative entry counts; coalesced watch updates never lose admission credits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Progress {
    pub(super) started: u64,
    pub(super) finished: u64,
}
