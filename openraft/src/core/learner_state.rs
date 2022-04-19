use tracing::Instrument;

use crate::core::MetricsProvider;
use crate::core::RaftCore;
use crate::core::ServerState;
use crate::engine::Command;
use crate::entry::EntryRef;
use crate::error::ExtractFatal;
use crate::error::Fatal;
use crate::raft::RaftMsg;
use crate::runtime::RaftRuntime;
use crate::summary::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Update;

/// Volatile state specific to a Raft node in learner state.
pub(crate) struct LearnerState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    pub(crate) core: &'a mut RaftCore<C, N, S>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> MetricsProvider<C::NodeId>
    for LearnerState<'a, C, N, S>
{
    // the non-leader state use the default impl of `get_leader_metrics_option`
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LearnerState<'a, C, N, S> {
    pub(crate) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        Self { core }
    }

    /// Run the learner loop.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="learner"))]
    pub(crate) async fn run(self) -> Result<(), Fatal<C::NodeId>> {
        self.learner_loop().await?;
        Ok(())
    }

    async fn learner_loop(mut self) -> Result<(), Fatal<C::NodeId>> {
        // report the new state before enter the loop
        self.core.report_metrics(Update::Update(None));

        loop {
            if !self.core.engine.state.server_state.is_learner() {
                return Ok(());
            }

            self.core.report_metrics_if_needed(&self);
            self.core.engine.metrics_flags.reset();

            let span = tracing::debug_span!("CHrx:LearnerState");
            let _ent = span.enter();

            tokio::select! {
                Some((msg,span)) = self.core.rx_api.recv() => {
                    self.handle_msg(msg).instrument(span).await?;
                },

                Some(update) = self.core.rx_compaction.recv() => {
                    self.core.update_snapshot_state(update);
                },

                Ok(_) = &mut self.core.rx_shutdown => self.core.set_target_state(ServerState::Shutdown),
            }
        }
    }

    // TODO(xp): define a handle_msg method in RaftCore that decides what to do by current State.
    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "learner", id=display(self.core.id)))]
    pub(crate) async fn handle_msg(&mut self, msg: RaftMsg<C, N, S>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::AppendEntries { rpc, tx } => {
                let _ = tx.send(self.core.handle_append_entries_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::RequestVote { rpc, tx } => {
                let _ = tx.send(self.core.handle_vote_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::InstallSnapshot { rpc, tx } => {
                let _ = tx.send(self.core.handle_install_snapshot_request(rpc).await.extract_fatal()?);
            }
            RaftMsg::CheckIsLeaderRequest { tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ClientWriteRequest { rpc: _, tx } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::Initialize { members, tx } => {
                let _ = tx.send(self.handle_init_with_config(members).await);
            }
            RaftMsg::AddLearner { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ChangeMembership { tx, .. } => {
                self.core.reject_with_forward_to_leader(tx);
            }
            RaftMsg::ExternalRequest { req } => {
                req(ServerState::Learner, &mut self.core.storage, &mut self.core.network);
            }
        };
        Ok(())
    }
}

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
