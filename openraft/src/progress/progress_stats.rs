#[derive(Clone, Debug, Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct ProgressStats {
    pub(crate) update_count: u64,
    pub(crate) move_count: u64,
    pub(crate) is_quorum_count: u64,
}
