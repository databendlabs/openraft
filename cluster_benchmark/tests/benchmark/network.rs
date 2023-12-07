//! A minimized store with least cost for benchmarking Openraft.

#[cfg(feature = "bt")] use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use openraft::async_trait::async_trait;
use openraft::error::InstallSnapshotError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::RemoteError;
use openraft::network::RPCOption;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::Config;
use openraft::Raft;

use crate::store::LogStore;
use crate::store::NodeId;
use crate::store::StateMachineStore;
use crate::store::TypeConfig as MemConfig;

pub type BenchRaft = Raft<MemConfig>;

#[derive(Clone)]
pub struct Router {
    pub(crate) table: Arc<Mutex<BTreeMap<NodeId, BenchRaft>>>,
}

impl Router {
    pub fn new() -> Self {
        Router {
            table: Default::default(),
        }
    }

    pub(crate) fn get_raft(&self, id: NodeId) -> BenchRaft {
        self.table.lock().unwrap().get(&id).unwrap().clone()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn new_cluster(&mut self, config: Arc<Config>, voter_ids: BTreeSet<NodeId>) -> anyhow::Result<()> {
        let mut rafts = BTreeMap::new();

        for id in voter_ids.iter() {
            let log_store = Arc::new(LogStore::default());
            let sm = Arc::new(StateMachineStore::new());

            let raft = Raft::new(*id, config.clone(), self.clone(), log_store, sm).await?;

            rafts.insert(*id, raft);
        }

        {
            let mut t = self.table.lock().unwrap();
            *t = rafts.clone();
        }

        tracing::info!("--- initializing single node cluster: {}", 0);
        rafts.get_mut(&0).unwrap().initialize(voter_ids.clone()).await?;
        let log_index = 1; // log 0: initial membership log

        tracing::info!(log_index, "--- wait for init node to become leader");

        for (id, s) in rafts.iter_mut() {
            tracing::info!(log_index, "--- wait init log: {}, index: {}", id, log_index);
            s.wait(timeout()).applied_index(Some(log_index), "init").await?;
        }

        Ok(())
    }
}

#[async_trait]
impl RaftNetworkFactory<MemConfig> for Router {
    type Network = Network;

    async fn new_client(&mut self, target: NodeId, _node: &()) -> Self::Network {
        Network {
            target,
            target_raft: self.table.lock().unwrap().get(&target).unwrap().clone(),
        }
    }
}

pub struct Network {
    target: NodeId,
    target_raft: BenchRaft,
}

#[async_trait]
impl RaftNetwork<MemConfig> for Network {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, RPCError<NodeId, (), RaftError<NodeId>>> {
        let resp = self.target_raft.append_entries(rpc).await.map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<MemConfig>,
        _option: RPCOption,
    ) -> Result<InstallSnapshotResponse<NodeId>, RPCError<NodeId, (), RaftError<NodeId, InstallSnapshotError>>> {
        let resp = self.target_raft.install_snapshot(rpc).await.map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RPCError<NodeId, (), RaftError<NodeId>>> {
        let resp = self.target_raft.vote(rpc).await.map_err(|e| RemoteError::new(self.target, e))?;
        Ok(resp)
    }
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}
