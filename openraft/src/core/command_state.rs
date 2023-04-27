#[derive(Debug, Clone)]
#[derive(Default)]
pub(crate) struct CommandState {
    /// The sequence number of the last finished sm command.
    pub(crate) finished_sm_seq: u64,
}
