use futures::future::TryFutureExt;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use maplit::btreeset;
use tokio::time::timeout;
use tokio::time::Duration;
use tracing::Instrument;
use tracing::Level;

use crate::core::LeaderState;
use crate::core::ServerState;
use crate::error::CheckIsLeaderError;
use crate::error::QuorumNotEnough;
use crate::error::RPCError;
use crate::error::Timeout;
use crate::progress::Progress;
use crate::quorum::QuorumSet;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::RaftRespTx;
use crate::replication::UpdateReplication;
use crate::LogId;
use crate::MessageSummary;
use crate::RPCTypes;
use crate::RaftNetwork;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;

impl<'a, C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> LeaderState<'a, C, N, S> {
    /// Handle `is_leader` requests.
    ///
    /// Spawn requests to all members of the cluster, include members being added in joint
    /// consensus. Each request will have a timeout, and we respond once we have a majority
    /// agreement from each config group. Most of the time, we will have a single uniform
    /// config group.
    ///
    /// From the spec (§8):
    /// Second, a leader must check whether it has been deposed before processing a read-only
    /// request (its information may be stale if a more recent leader has been elected). Raft
    /// handles this by having the leader exchange heartbeat messages with a majority of the
    /// cluster before responding to read-only requests.
    #[tracing::instrument(level = "trace", skip(self, tx))]
    pub(super) async fn handle_check_is_leader_request(&mut self, tx: RaftRespTx<(), CheckIsLeaderError<C::NodeId>>) {
        // Setup sentinel values to track when we've received majority confirmation of leadership.

        let em = &self.core.engine.state.membership_state.effective;
        let mut granted = btreeset! {self.core.id};

        if em.is_quorum(granted.iter()) {
            let _ = tx.send(Ok(()));
            return;
        }

        // Spawn parallel requests, all with the standard timeout for heartbeats.
        let mut pending = FuturesUnordered::new();

        let voter_progresses = if let Some(l) = &self.core.engine.state.leader {
            l.progress
                .iter()
                .filter(|(id, _v)| l.progress.is_voter(id) == Some(true))
                .copied()
                .collect::<Vec<_>>()
        } else {
            unreachable!("it has to be a leader!!!");
        };

        for (target, matched) in voter_progresses {
            if target == self.core.id {
                continue;
            }

            let rpc = AppendEntriesRequest {
                vote: self.core.engine.state.vote,
                prev_log_id: matched,
                entries: vec![],
                leader_commit: self.core.engine.state.committed,
            };

            let my_id = self.core.id;
            let target_node = self.core.engine.state.membership_state.effective.get_node(&target).cloned();
            let mut network = self.core.network.connect(target, target_node.as_ref()).await;

            let ttl = Duration::from_millis(self.core.config.heartbeat_interval);

            let task = tokio::spawn(
                async move {
                    let outer_res = timeout(ttl, network.send_append_entries(rpc)).await;
                    match outer_res {
                        Ok(append_res) => match append_res {
                            Ok(x) => Ok((target, x)),
                            Err(err) => Err((target, err)),
                        },
                        Err(_timeout) => {
                            let timeout_err = Timeout {
                                action: RPCTypes::AppendEntries,
                                id: my_id,
                                target,
                                timeout: ttl,
                            };

                            Err((target, RPCError::Timeout(timeout_err)))
                        }
                    }
                }
                // TODO(xp): add target to span
                .instrument(tracing::debug_span!("SPAWN_append_entries")),
            )
            .map_err(move |err| (target, err));

            pending.push(task);
        }

        // Handle responses as they return.
        while let Some(res) = pending.next().await {
            let (target, data) = match res {
                Ok(Ok(res)) => res,
                Ok(Err((target, err))) => {
                    tracing::error!(target=display(target), error=%err, "timeout while confirming leadership for read request");
                    continue;
                }
                Err((target, err)) => {
                    tracing::error!(target = display(target), "{}", err);
                    continue;
                }
            };

            // If we receive a response with a greater term, then revert to follower and abort this request.
            if let AppendEntriesResponse::HigherVote(vote) = data {
                assert!(vote > self.core.engine.state.vote);
                self.core.engine.state.vote = vote;
                // TODO(xp): deal with storage error
                self.core.save_vote().await.unwrap();
                // TODO(xp): if receives error about a higher term, it should stop at once?
                self.core.set_target_state(ServerState::Follower);
            }

            granted.insert(target);

            let mem = &self.core.engine.state.membership_state.effective;
            if mem.is_quorum(granted.iter()) {
                let _ = tx.send(Ok(()));
                return;
            }
        }

        // If we've hit this location, then we've failed to gather needed confirmations due to
        // request failures.

        let _ = tx.send(Err(QuorumNotEnough {
            cluster: self.core.engine.state.membership_state.effective.membership.summary(),
            got: granted,
        }
        .into()));
    }

    /// Begin replicating upto the given log id.
    ///
    /// It does not block until the entry is committed or actually sent out.
    /// It merely broadcasts a signal to inform the replication threads.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(super) fn replicate_entry(&mut self, log_id: LogId<C::NodeId>) {
        if tracing::enabled!(Level::DEBUG) {
            for node_id in self.nodes.keys() {
                tracing::debug!(node_id = display(node_id), log_id = display(log_id), "replicate_entry");
            }
        }

        for node in self.nodes.values() {
            let _ = node.repl_tx.send(UpdateReplication {
                last_log_id: Some(log_id),
                committed: self.core.engine.state.committed,
            });
        }
    }
}
