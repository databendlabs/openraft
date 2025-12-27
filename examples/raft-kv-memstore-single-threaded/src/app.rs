use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::channel::oneshot;
use openraft_network_v1::RaftV1;

use crate::NodeId;
use crate::StateMachineStore;
use crate::TypeConfig;
use crate::api;
use crate::router::Router;
use crate::typ::Raft;

pub type Path = String;
pub type Payload = String;
pub type ResponseTx = oneshot::Sender<String>;
pub type RequestTx = mpsc::Sender<(Path, Payload, ResponseTx)>;
pub type RequestRx = mpsc::Receiver<(Path, Payload, ResponseTx)>;

/// Representation of an application state.
pub struct App {
    pub id: NodeId,
    pub raft: RaftV1<TypeConfig>,

    /// Receive application requests, Raft protocol request or management requests.
    pub rx: RequestRx,
    pub router: Router,

    pub state_machine: Rc<StateMachineStore>,
}

impl App {
    pub fn new(id: NodeId, raft: Raft, router: Router, state_machine: Rc<StateMachineStore>) -> Self {
        let (tx, rx) = mpsc::channel(1024);

        {
            let mut targets = router.targets.borrow_mut();
            targets.insert(id, tx);
        }

        let raft = RaftV1::new(raft);

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
            let (path, payload, response_tx) = self.rx.next().await?;

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
