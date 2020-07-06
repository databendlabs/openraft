use crate::{AppData, AppDataResponse, AppError, RaftNetwork, RaftStorage};
use crate::raft::Entry;
use crate::core::RaftCore;

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> RaftCore<D, R, E, N, S> {
    /// Apply the given log entry to the state machine.
    pub(super) async fn apply_entry_to_state_machine(&mut self, entry: &Entry<D>) -> Result<R, E> {
        // First, we just ensure that we apply any outstanding up to, but not including, the index
        // of the given entry. We need to be able to return the data response from applying this
        // entry to the state machine.
        //
        // Note that this would only ever happen if a node had unapplied logs from before becoming leader.
        let expected_next_index = self.last_applied + 1;
        if entry.index != expected_next_index {
            let entries = self.storage.get_log_entries(expected_next_index, entry.index).await.map_err(|err| self.map_fatal_storage_result(err))?;
            self.storage.replicate_to_state_machine(&entries).await.map_err(|err| self.map_fatal_storage_result(err))?;
            if let Some(entry) = entries.iter().last() {
                self.last_applied = entry.index;
            }
        }

        // Apply this entry to the state machine and return its data response.
        let res = self.storage.apply_entry_to_state_machine(entry).await.map_err(|err| self.map_fatal_storage_result(err))?;
        self.last_applied = entry.index;
        Ok(res)
    }
}
