use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::api;
use crate::encode;
use crate::router::NodeMessage;
use crate::router::NodeRx;
use crate::router::NodeTx;
use crate::router::Router;
use crate::typ;
use crate::GroupId;
use crate::NodeId;
use crate::StateMachineStore;

/// A Node manages multiple Raft groups on the same physical node.
///
/// All groups share ONE connection to this node.
/// The Node dispatches incoming messages to the correct group based on group_id.
pub struct Node {
    pub node_id: NodeId,
    pub groups: BTreeMap<GroupId, GroupApp>,
    pub rx: NodeRx,
    pub router: Router,
}

impl Node {
    pub fn new(node_id: NodeId, router: Router) -> (Self, NodeTx) {
        let (tx, rx) = mpsc::unbounded_channel();

        // Register this node's shared connection
        router.register_node(node_id, tx.clone());

        let node = Self {
            node_id,
            groups: BTreeMap::new(),
            rx,
            router,
        };

        (node, tx)
    }

    /// Add a Raft group to this node.
    pub fn add_group(&mut self, group_id: GroupId, raft: typ::Raft, state_machine: Arc<StateMachineStore>) {
        let app = GroupApp {
            node_id: self.node_id,
            group_id: group_id.clone(),
            raft,
            state_machine,
        };
        self.groups.insert(group_id, app);
    }

    /// Get a Raft instance by group_id.
    pub fn get_raft(&self, group_id: &GroupId) -> Option<&typ::Raft> {
        self.groups.get(group_id).map(|g| &g.raft)
    }

    /// Run the node message dispatcher.
    /// Routes incoming messages to the correct group based on group_id.
    pub async fn run(mut self) -> Option<()> {
        loop {
            let msg = self.rx.recv().await?;

            let NodeMessage {
                group_id,
                path,
                payload,
                response_tx,
            } = msg;

            // Find the target group
            let group = match self.groups.get_mut(&group_id) {
                Some(g) => g,
                None => {
                    let _ = response_tx.send(encode::<Result<(), typ::RaftError>>(Err(typ::RaftError::Fatal(
                        openraft::error::Fatal::Stopped,
                    ))));
                    continue;
                }
            };

            // Dispatch to the group
            let res = match path.as_str() {
                // Application API
                "/app/write" => api::write(group, payload).await,
                "/app/read" => api::read(group, payload).await,

                // Raft API
                "/raft/append" => api::append(group, payload).await,
                "/raft/snapshot" => api::snapshot(group, payload).await,
                "/raft/vote" => api::vote(group, payload).await,
                "/raft/transfer_leader" => api::transfer_leader(group, payload).await,

                // Management API
                "/mng/add-learner" => api::add_learner(group, payload).await,
                "/mng/change-membership" => api::change_membership(group, payload).await,
                "/mng/init" => api::init(group).await,
                "/mng/metrics" => api::metrics(group).await,

                _ => {
                    tracing::warn!("unknown path: {}", path);
                    encode::<Result<(), typ::RaftError>>(Err(typ::RaftError::Fatal(openraft::error::Fatal::Stopped)))
                }
            };

            let _ = response_tx.send(res);
        }
    }
}

/// A single Raft group's application context.
pub struct GroupApp {
    pub node_id: NodeId,
    pub group_id: GroupId,
    pub raft: typ::Raft,
    pub state_machine: Arc<StateMachineStore>,
}
