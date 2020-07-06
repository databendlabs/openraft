//! Development and testing utilities.

use std::{
    collections::BTreeMap,
    time::Duration,
};

use actix::prelude::*;
use actix_raft::{
    Raft, NodeId,
    messages::{
        AppendEntriesRequest, AppendEntriesResponse,
        InstallSnapshotRequest, InstallSnapshotResponse,
        VoteRequest, VoteResponse,
    },
    network::RaftNetwork,
    metrics::{RaftMetrics, State},
};
use log::{debug};

use crate::fixtures::memory_storage::{MemoryStorage, MemoryStorageData, MemoryStorageError, MemoryStorageResponse};

const ERR_ROUTING_FAILURE: &str = "Routing failures are not allowed in tests.";

/// A concrete Raft type used during testing.
pub type MemRaft = Raft<MemoryStorageData, MemoryStorageResponse, MemoryStorageError, RaftRouter, MemoryStorage>;

//////////////////////////////////////////////////////////////////////////////////////////////////
// RaftRouter ////////////////////////////////////////////////////////////////////////////////////

/// An actor which emulates a network transport and implements the `RaftNetwork` trait.
#[derive(Default)]
pub struct RaftRouter {
    pub routing_table: BTreeMap<NodeId, Addr<MemRaft>>,
    pub metrics: BTreeMap<NodeId, RaftMetrics>,
    /// Nodes which are isolated can neither send nor receive frames.
    isolated_nodes: Vec<NodeId>,
    /// The count of all messages which have passed through this system.
    routed: (u64, u64, u64, u64), // AppendEntries, Vote, InstallSnapshot, Other.
}

impl RaftRouter {
    /// Create a new instance.
    pub fn new() -> Self {
        Default::default()
    }

    /// Isolate the network of the specified node.
    pub fn isolate_node(&mut self, id: NodeId) {
        debug!("Isolating network for node {}.", &id);
        self.isolated_nodes.push(id);
    }

    /// Restore the network of the specified node.
    pub fn restore_node(&mut self, id: NodeId) {
        if let Some((idx, _)) = self.isolated_nodes.iter().enumerate().find(|(_, e)| *e == &id) {
            debug!("Restoring network for node {}.", &id);
            self.isolated_nodes.remove(idx);
        }
    }
}

impl Actor for RaftRouter {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_interval(Duration::from_secs(1), |act, _| {
            debug!("RaftRouter [AppendEntries={}, Vote={}, InstallSnapshot={}, Other={}]", act.routed.0, act.routed.1, act.routed.2, act.routed.3);
        });
    }
}

//////////////////////////////////////////////////////////////////////////////
// Impl RaftNetwork //////////////////////////////////////////////////////////

impl RaftNetwork<MemoryStorageData> for RaftRouter {}

impl Handler<AppendEntriesRequest<MemoryStorageData>> for RaftRouter {
    type Result = ResponseActFuture<Self, AppendEntriesResponse, ()>;

    fn handle(&mut self, msg: AppendEntriesRequest<MemoryStorageData>, _: &mut Self::Context) -> Self::Result {
        self.routed.0 += 1;
        let addr = self.routing_table.get(&msg.target).unwrap();
        if self.isolated_nodes.contains(&msg.target) || self.isolated_nodes.contains(&msg.leader_id) {
            return Box::new(fut::err(()));
        }
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<VoteRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, VoteResponse, ()>;

    fn handle(&mut self, msg: VoteRequest, _: &mut Self::Context) -> Self::Result {
        self.routed.1 += 1;
        let addr = self.routing_table.get(&msg.target).unwrap();
        if self.isolated_nodes.contains(&msg.target) || self.isolated_nodes.contains(&msg.candidate_id) {
            return Box::new(fut::err(()));
        }
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

impl Handler<InstallSnapshotRequest> for RaftRouter {
    type Result = ResponseActFuture<Self, InstallSnapshotResponse, ()>;

    fn handle(&mut self, msg: InstallSnapshotRequest, _: &mut Self::Context) -> Self::Result {
        self.routed.2 += 1;
        let addr = self.routing_table.get(&msg.target).unwrap();
        if self.isolated_nodes.contains(&msg.target) || self.isolated_nodes.contains(&msg.leader_id) {
            return Box::new(fut::err(()));
        }
        Box::new(fut::wrap_future(addr.send(msg))
            .map_err(|_, _, _| panic!(ERR_ROUTING_FAILURE))
            .and_then(|res, _, _| fut::result(res)))
    }
}

//////////////////////////////////////////////////////////////////////////////
// RaftMetrics ///////////////////////////////////////////////////////////////

impl Handler<RaftMetrics> for RaftRouter {
    type Result = ();

    fn handle(&mut self, msg: RaftMetrics, _: &mut Context<Self>) -> Self::Result {
        self.routed.3 += 1;
        debug!("Metrics: node={} state={:?} leader={:?} term={} index={} applied={} cfg={{join={} members={:?} non_voters={:?} removing={:?}}}",
            msg.id, msg.state, msg.current_leader, msg.current_term, msg.last_log_index, msg.last_applied,
            msg.membership_config.is_in_joint_consensus, msg.membership_config.members,
            msg.membership_config.non_voters, msg.membership_config.removing,
        );
        self.metrics.insert(msg.id, msg);
    }
}

//////////////////////////////////////////////////////////////////////////////
// Test Commands /////////////////////////////////////////////////////////////

/// Get the current leader of the cluster from the perspective of the Raft metrics.
///
/// A return value of Ok(None) indicates that the current leader is unknown or the cluster hasn't
/// come to consensus on the leader yet.
pub struct GetCurrentLeader;

impl Message for GetCurrentLeader {
    type Result = Result<Option<NodeId>, ()>;
}

impl Handler<GetCurrentLeader> for RaftRouter {
    type Result = Result<Option<NodeId>, ()>;

    fn handle(&mut self, _: GetCurrentLeader, _: &mut Self::Context) -> Self::Result {
        self.routed.3 += 1;
        let leader_opt = self.metrics.values()
            .filter(|e| !self.isolated_nodes.contains(&e.id))
            .find(|e| &e.state == &State::Leader);

        if let Some(leader) = leader_opt {
            let has_consensus = self.metrics.values()
                .filter(|e| !self.isolated_nodes.contains(&e.id) && leader.membership_config.contains(&e.id))
                .all(|e| e.current_leader == Some(leader.id) && e.current_term == leader.current_term);

            if has_consensus {
                Ok(Some(leader.id))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

// Register //////////////////////////////////////////////////////////////////

#[derive(Message)]
pub struct Register {
    pub id: NodeId,
    pub addr: Addr<MemRaft>,
}

impl Handler<Register> for RaftRouter {
    type Result = ();

    fn handle(&mut self, msg: Register, _: &mut Self::Context) -> Self::Result {
        self.routed.3 += 1;
        self.routing_table.insert(msg.id, msg.addr);
    }
}

// RemoveNodeFromCluster /////////////////////////////////////////////////////

/// Remove the specified node from the cluster.
///
/// This operation will only succeed if the target node is in NonVoter state, and does not appear
/// in the config of the current leader.
pub struct RemoveNodeFromCluster {
    pub id: NodeId,
}

impl Message for RemoveNodeFromCluster {
    type Result = Result<(), String>;
}

impl Handler<RemoveNodeFromCluster> for RaftRouter {
    type Result = Result<(), String>;

    fn handle(&mut self, msg: RemoveNodeFromCluster, _: &mut Self::Context) -> Self::Result {
        self.routed.3 += 1;
        let leader_opt = self.metrics.values()
            .filter(|e| !self.isolated_nodes.contains(&e.id))
            .find(|e| &e.state == &State::Leader);

        if let Some(leader) = leader_opt {
            let leader_knows_target = leader.membership_config.contains(&msg.id);
            if !leader_knows_target {
                self.routing_table.remove(&msg.id);
                if let Some((idx, _)) = self.isolated_nodes.iter().enumerate().find(|(_, e)| *e == &msg.id) {
                    self.isolated_nodes.remove(idx);
                }
                self.metrics.remove(&msg.id);
                Ok(())
            } else {
                Err(String::from("Cluster leader has the target node in its current config."))
            }
        } else {
            Err(String::from("Cluster has no current leader, can not verify that it is safe to remove node."))
        }
    }
}

// ExecuteInRaftRouter ///////////////////////////////////////////////////////

pub struct ExecuteInRaftRouter(pub Box<dyn FnOnce(&mut RaftRouter, &mut Context<RaftRouter>) + Send + 'static>);

impl Message for ExecuteInRaftRouter {
    type Result = Result<(), ()>;
}

impl Handler<ExecuteInRaftRouter> for RaftRouter {
    type Result = Result<(), ()>;

    fn handle(&mut self, msg: ExecuteInRaftRouter, ctx: &mut Self::Context) -> Self::Result {
        self.routed.3 += 1;
        msg.0(self, ctx);
        Ok(())
    }
}
