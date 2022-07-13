use std::collections::BTreeMap;

use tokio::sync::mpsc;
use tracing::Instrument;
use tracing::Span;

use crate::core::RaftCore;
use crate::core::ServerState;
use crate::engine::Command;
use crate::error::Fatal;
use crate::raft::RaftMsg;
use crate::raft_types::RaftLogId;
use crate::replication::ReplicaEvent;
use crate::replication::ReplicationStream;
use crate::replication::UpdateReplication;
use crate::runtime::RaftRuntime;
use crate::summary::MessageSummary;
use crate::Entry;
use crate::EntryPayload;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Update;

/// Volatile state specific to a Raft node in leader state.
pub(crate) struct LeaderState<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> {
    pub(super) core: &'a mut RaftCore<C, N, S>,

    /// A mapping of node IDs the replication state of the target node.
    pub(super) nodes: BTreeMap<C::NodeId, ReplicationStream<C::NodeId>>,

    /// The stream of events coming from replication streams.
    #[allow(clippy::type_complexity)]
    pub(super) replication_rx: mpsc::UnboundedReceiver<(ReplicaEvent<C::NodeId, S::SnapshotData>, Span)>,

    /// The cloneable sender channel for replication stream events.
    #[allow(clippy::type_complexity)]
    pub(super) replication_tx: mpsc::UnboundedSender<(ReplicaEvent<C::NodeId, S::SnapshotData>, Span)>,
}

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    /// Create a new instance.
    pub(crate) fn new(core: &'a mut RaftCore<C, N, S>) -> Self {
        let (replication_tx, replication_rx) = mpsc::unbounded_channel();
        Self {
            core,
            nodes: BTreeMap::new(),
            replication_tx,
            replication_rx,
        }
    }

    /// Transition to the Raft leader state.
    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id), raft_state="leader"))]
    pub(crate) async fn run(mut self) -> Result<(), Fatal<C::NodeId>> {
        // Setup state as leader.
        self.core.last_heartbeat = None;
        self.core.next_election_timeout = None;
        debug_assert!(self.core.engine.state.vote.committed);

        // Spawn replication streams for followers and learners.

        let targets = {
            let mem = &self.core.engine.state.membership_state.effective;

            let node_ids = mem.nodes().map(|(&nid, _)| nid);
            node_ids.filter(|elem| elem != &self.core.id).collect::<Vec<_>>()
        };

        // TODO(xp): make this Engine::Command driven.
        for target in targets {
            let state = self.spawn_replication_stream(target).await;
            self.nodes.insert(target, state);
        }

        // Commit the initial entry when new leader established.
        self.write_entry(EntryPayload::Blank, None).await?;

        self.leader_loop().await?;

        Ok(())
    }

    #[tracing::instrument(level="debug", skip(self), fields(id=display(self.core.id)))]
    pub(self) async fn leader_loop(mut self) -> Result<(), Fatal<C::NodeId>> {
        // report the leader metrics every time there came to a new leader
        // if not `report_metrics` before the leader loop, the leader metrics may not be updated cause no coming event.
        let replication_metrics = if let Some(l) = &self.core.leader_data {
            l.replication_metrics.clone()
        } else {
            unreachable!("it has to be a leader!!!");
        };
        self.core.report_metrics(Update::Update(Some(replication_metrics)));

        loop {
            if !self.core.engine.state.server_state.is_leader() {
                tracing::info!(
                    "id={} state becomes: {:?}",
                    self.core.id,
                    self.core.engine.state.server_state
                );

                // implicit drop replication_rx
                // notify to all nodes DO NOT send replication event any more.
                return Ok(());
            }

            self.core.flush_metrics();

            tokio::select! {
                Some((msg,span)) = self.core.rx_api.recv() => {
                    self.handle_msg(msg).instrument(span).await?;
                },

                Some(internal_msg) = self.core.rx_internal.recv() => {
                    tracing::info!("leader recv from rx_internal: {:?}", internal_msg);
                    self.core.handle_internal_msg(internal_msg).await?;
                }

                Some((event, span)) = self.replication_rx.recv() => {
                    tracing::info!("leader recv from replication_rx: {:?}", event.summary());
                    self.handle_replica_event(event).instrument(span).await?;
                }

                Ok(_) = &mut self.core.rx_shutdown => {
                    tracing::info!("leader recv from rx_shudown");
                    self.core.set_target_state(ServerState::Shutdown);
                }
            }
        }
    }

    #[tracing::instrument(level = "debug", skip(self, msg), fields(state = "leader", id=display(self.core.id)))]
    pub async fn handle_msg(&mut self, msg: RaftMsg<C, N, S>) -> Result<(), Fatal<C::NodeId>> {
        tracing::debug!("recv from rx_api: {}", msg.summary());

        match msg {
            RaftMsg::CheckIsLeaderRequest { tx } => {
                self.handle_check_is_leader_request(tx).await;
            }
            RaftMsg::ClientWriteRequest { rpc, tx } => {
                self.write_entry(rpc.payload, Some(tx)).await?;
            }
            RaftMsg::AddLearner { id, node, tx } => {
                self.add_learner(id, node, tx).await?;
            }
            RaftMsg::ChangeMembership {
                changes,
                when,
                turn_to_learner,
                tx,
            } => {
                self.change_membership(changes, when, turn_to_learner, tx).await?;
            }

            _ => {
                // Call the default handler for non-leader-specific msg
                self.core.handle_api_msg(msg).await?;
            }
        };

        Ok(())
    }
}

/// impl Runtime for LeaderState
#[async_trait::async_trait]
impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftRuntime<C> for LeaderState<'a, C, N, S> {
    async fn run_command<'e, Ent>(
        &mut self,
        input_entries: &'e [Ent],
        curr: &mut usize,
        cmd: &Command<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>>
    where
        Ent: RaftLogId<C::NodeId> + Sync + Send + 'e,
        &'e Ent: Into<Entry<C>>,
    {
        // Run leader specific commands or pass non leader specific commands to self.core.
        match cmd {
            Command::ReplicateCommitted { committed } => {
                for node in self.nodes.values() {
                    let _ = node.repl_tx.send(UpdateReplication {
                        last_log_id: None,
                        committed: *committed,
                    });
                }
            }
            Command::ReplicateInputEntries { range } => {
                if let Some(last) = range.clone().last() {
                    self.replicate_entry(*input_entries[last].get_log_id());
                }
            }
            Command::UpdateReplicationStreams { remove, add } => {
                for (node_id, _matched) in remove.iter() {
                    self.remove_replication(*node_id).await;
                }
                for (node_id, _matched) in add.iter() {
                    let state = self.spawn_replication_stream(*node_id).await;
                    self.nodes.insert(*node_id, state);
                }
            }
            _ => self.core.run_command(input_entries, curr, cmd).await?,
        }

        Ok(())
    }
}
