use crate::core::LearnerState;
use crate::engine::Command;
use crate::entry::EntryRef;
use crate::runtime::RaftRuntime;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

#[async_trait::async_trait]
impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for LearnerState<'a, C, N, S> {
    async fn run_command<'p>(
        &mut self,
        input_entries: &[EntryRef<'p, C>],
        curr: &mut usize,
        cmd: &Command<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        // A learner has no special cmd impl, pass all to core.
        self.core.run_command(input_entries, curr, cmd).await
    }
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LearnerState<'a, C, N, S> {
    pub(crate) async fn run_engine_commands<'p>(
        &mut self,
        input_entries: &[EntryRef<'p, C>],
    ) -> Result<(), StorageError<C::NodeId>> {
        let mut curr = 0;
        let it = self.core.engine.commands.drain(..).collect::<Vec<_>>();
        for cmd in it {
            self.run_command(input_entries, &mut curr, &cmd).await?;
        }

        Ok(())
    }
}
