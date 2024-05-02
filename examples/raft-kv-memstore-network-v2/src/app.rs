use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::api;
use crate::router::Router;
use crate::typ;
use crate::NodeId;
use crate::StateMachineStore;

pub type Path = String;
pub type Payload = String;
pub type ResponseTx = oneshot::Sender<String>;
pub type RequestTx = mpsc::UnboundedSender<(Path, Payload, ResponseTx)>;

/// Representation of an application state.
pub struct App {
    pub id: NodeId,
    pub raft: typ::Raft,

    /// Receive application requests, Raft protocol request or management requests.
    pub rx: mpsc::UnboundedReceiver<(Path, Payload, ResponseTx)>,
    pub router: Router,

    pub state_machine: Arc<StateMachineStore>,
}

impl App {
    pub fn new(id: NodeId, raft: typ::Raft, router: Router, state_machine: Arc<StateMachineStore>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        {
            let mut targets = router.targets.lock().unwrap();
            targets.insert(id, tx);
        }

        Self {
            id,
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

            response_tx.send(res).unwrap();
        }
    }
}
