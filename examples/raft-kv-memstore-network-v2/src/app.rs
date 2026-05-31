use futures::StreamExt;
use futures::channel::mpsc;
use futures::channel::oneshot;

use crate::NodeId;
use crate::Raft;
use crate::StateMachineStore;
use crate::api;
use crate::router::Router;

pub type Path = String;
pub type Payload = String;
pub type ResponseTx = oneshot::Sender<String>;
pub type RequestTx = mpsc::Sender<(Path, Payload, ResponseTx)>;
pub type RequestRx = mpsc::Receiver<(Path, Payload, ResponseTx)>;

/// Representation of an application state.
pub struct App {
    pub id: NodeId,
    pub raft: Raft,
    pub raft_addr: String,

    /// Receive application and management requests from the test router.
    pub rx: RequestRx,
    pub router: Router,

    pub state_machine: StateMachineStore,
}

impl App {
    pub fn new(id: NodeId, raft: Raft, router: Router, state_machine: StateMachineStore, raft_addr: String) -> Self {
        let (tx, rx) = mpsc::channel(1024);

        {
            let mut targets = router.targets.lock().unwrap();
            targets.insert(id, tx);
        }

        Self {
            id,
            raft,
            raft_addr,
            rx,
            router,
            state_machine,
        }
    }

    pub async fn run(mut self) -> Option<()> {
        loop {
            let (path, payload, response_tx) = self.rx.next().await?;

            let res = match path.as_str() {
                // Application API
                "/app/write" => api::write(&mut self, payload).await,
                "/app/read" => api::read(&mut self, payload).await,

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
