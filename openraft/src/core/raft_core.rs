use crate::core::RaftCore;
use crate::engine::Command;
use crate::entry::EntryRef;
use crate::runtime::RaftRuntime;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    // TODO:
    #[allow(dead_code)]
    async fn run_engine_commands<'p>(
        &mut self,
        input_entries: &[EntryRef<'p, C>],
    ) -> Result<(), StorageError<C::NodeId>> {
        let mut curr = 0;
        let it = self.engine.commands.drain(..).collect::<Vec<_>>();
        for cmd in it {
            self.run_command(input_entries, &mut curr, &cmd).await?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for RaftCore<C, N, S> {
    async fn run_command<'p>(
        &mut self,
        input_ref_entries: &[EntryRef<'p, C>],
        cur: &mut usize,
        cmd: &Command<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // Run non-role-specific command.
        match cmd {
            Command::AppendInputEntries { range } => {
                let entry_refs = &input_ref_entries[range.clone()];

                let mut entries = Vec::with_capacity(entry_refs.len());
                for ent in entry_refs.iter() {
                    entries.push(ent.into())
                }

                // Build a slice of references.
                let entry_refs = entries.iter().collect::<Vec<_>>();

                self.storage.append_to_log(&entry_refs).await?
            }
            Command::MoveInputCursorBy { n } => *cur += n,
            Command::SaveVote { .. } => {}
            Command::PurgeAppliedLog { .. } => {}
            Command::DeleteConflictLog { .. } => {}
            Command::BuildSnapshot { .. } => {}
            Command::SendVote { .. } => {}
            Command::Commit { .. } => {}
            Command::ReplicateInputEntries { .. } => {
                unreachable!("leader specific command")
            }
            Command::UpdateMembership { .. } => {}
        }

        Ok(())
    }
}
