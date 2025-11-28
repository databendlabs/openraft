use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::api;
use crate::router::Router;
use crate::typ;
use crate::GroupId;
use crate::NodeId;
use crate::StateMachineStore;

pub type Path = String;
pub type Payload = String;
pub type ResponseTx = oneshot::Sender<String>;
pub type RequestTx = mpsc::UnboundedSender<(Path, Payload, ResponseTx)>;

/// Each App instance handles requests for one specific (node_id, group_id) combination.
pub struct App {
    pub node_id: NodeId,
    pub group_id: GroupId,
    /// The Raft instance for this group
    pub raft: typ::Raft,
    pub rx: mpsc::UnboundedReceiver<(Path, Payload, ResponseTx)>,
    pub router: Router,
    pub state_machine: Arc<StateMachineStore>,
}

impl App {
    pub fn new(
        node_id: NodeId,
        group_id: GroupId,
        raft: typ::Raft,
        router: Router,
        state_machine: Arc<StateMachineStore>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        router.register(node_id, group_id.clone(), tx);

        Self {
            node_id,
            group_id,
            raft,
            rx,
            router,
            state_machine,
        }
    }

    pub async fn run(mut self) -> Option<()> {
        loop {
            let (path, payload, response_tx) = self.rx.recv().await?;

            let res = match path.as_str() {
                // Application API
                "/app/write" => api::write(&mut self, payload).await,
                "/app/read" => api::read(&mut self, payload).await,

                // Raft API
                "/raft/append" => api::append(&mut self, payload).await,
                "/raft/snapshot" => api::snapshot(&mut self, payload).await,
                "/raft/vote" => api::vote(&mut self, payload).await,

                // Management API
                "/mng/add-learner" => api::add_learner(&mut self, payload).await,
                "/mng/change-membership" => api::change_membership(&mut self, payload).await,
                "/mng/init" => api::init(&mut self).await,
                "/mng/metrics" => api::metrics(&mut self).await,

                _ => panic!("unknown path: {}", path),
            };

            let _ = response_tx.send(res);
        }
    }
}
