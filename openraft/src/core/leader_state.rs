use crate::core::LeaderState;
use crate::engine::Command;
use crate::entry::EntryRef;
use crate::entry::RaftEntry;
use crate::runtime::RaftRuntime;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

#[async_trait::async_trait]
impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for LeaderState<'a, C, N, S> {
    async fn run_command<'p>(
        &mut self,
        input_entries: &[EntryRef<'p, C>],
        curr: &mut usize,
        cmd: &Command<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Run leader specific commands or pass non leader specific commands to self.core.
        match cmd {
            Command::Commit { ref upto } => {
                for ent in input_entries.iter() {
                    let log_id = &ent.log_id;
                    if log_id <= upto {
                        self.client_request_post_commit(log_id.index).await?;
                    } else {
                        break;
                    }
                }
            }
            Command::ReplicateInputEntries { range } => {
                for i in range.clone() {
                    self.replicate_entry(*input_entries[i].get_log_id());
                }
            }
            Command::UpdateMembership { .. } => {
                // TODO: rebuild replication streams. not used yet. Currently replication stream management is done
                // before this step.
            }
            _ => self.core.run_command(input_entries, curr, cmd).await?,
        }

        Ok(())
    }
}
