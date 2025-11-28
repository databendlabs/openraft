use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::api;
use crate::router::Router;
use crate::typ;
use crate::NodeId;
use crate::ShardId;
use crate::StateMachineStore;

/// Type alias for the request path (e.g., "/raft/vote").
pub type Path = String;

/// Type alias for the serialized request payload.
pub type Payload = String;

/// Type alias for the response channel.
pub type ResponseTx = oneshot::Sender<String>;

/// Type alias for the request channel sender.
pub type RequestTx = mpsc::UnboundedSender<(Path, Payload, ResponseTx)>;

/// Application handler for a single shard on a single node.
///
/// Each App instance handles:
/// - Raft protocol RPCs (vote, append_entries, snapshot)
/// - Client application requests (read, write)
/// - Management operations (add_learner, change_membership)
pub struct App {
    pub node_id: NodeId,

    pub shard_id: ShardId,

    pub raft: typ::Raft,

    pub rx: mpsc::UnboundedReceiver<(Path, Payload, ResponseTx)>,

    /// The shared message router.
    pub router: Router,

    /// The state machine store for this shard.
    pub state_machine: Arc<StateMachineStore>,
}

impl App {
    /// Create a new App instance and register it with the router.
    pub fn new(
        node_id: NodeId,
        shard_id: ShardId,
        raft: typ::Raft,
        router: Router,
        state_machine: Arc<StateMachineStore>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        // Register this app with the router so it can receive messages.
        router.register(node_id, shard_id.clone(), tx);

        Self {
            node_id,
            shard_id,
            raft,
            rx,
            router,
            state_machine,
        }
    }

    pub async fn run(mut self) -> Option<()> {
        tracing::info!(
            node_id = %self.node_id,
            shard_id = %self.shard_id,
            "starting app event loop"
        );

        loop {
            let (path, payload, response_tx) = self.rx.recv().await?;

            let res = match path.as_str() {
                // Application API
                "/app/write" => api::write(&mut self, payload).await,
                "/app/read" => api::read(&mut self, payload).await,

                // Raft Protocol API
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
