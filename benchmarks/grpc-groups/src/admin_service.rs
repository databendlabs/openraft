use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

use futures::future::try_join_all;
use openraft::Config;
use openraft::async_runtime::WatchReceiver;
use raft_kv_memstore_grpc::LogStore;
use raft_kv_memstore_grpc::StateMachineStore;
use raft_kv_memstore_grpc::protobuf as raftpb;
use tokio::time::Instant;
use tonic::Request;
use tonic::Response;
use tonic::Status;

use crate::Raft;
use crate::group_network::GroupNetwork;
use crate::protobuf as pb;
use crate::protobuf::admin_server::Admin;
use crate::ready::is_ready_leader;
use crate::rejoins::Rejoins;
use crate::router::Router;
use crate::takeovers::Takeovers;
use crate::window::Window;

/// How long a node waits for the groups it initialized to elect it and commit an entry.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a node watches a group whose leader is about to fail, unless the driver disarms the
/// watch when `failover` ends. The watch starts before the fault, so it lasts longer than the
/// 30 s that the driver waits after it.
const TAKEOVER_LIMIT: Duration = Duration::from_secs(40);

/// Carries out the driver's requests on one node.
pub(crate) struct AdminService {
    id: u64,
    config: Arc<Config>,
    router: Router,

    /// This node's member of each group, by group id, set at setup.
    groups: Arc<OnceLock<Vec<Raft>>>,

    /// Every node of the run, by node id, as the membership of each group holds it, set at setup.
    nodes: OnceLock<BTreeMap<u64, raftpb::Node>>,

    window: Mutex<Option<Window>>,
    takeovers: Takeovers,

    /// Shared with the Raft server, which reports the AppendEntries requests the members accept.
    rejoins: Rejoins,
}

impl AdminService {
    pub(crate) fn new(
        id: u64,
        config: Arc<Config>,
        router: Router,
        groups: Arc<OnceLock<Vec<Raft>>>,
        rejoins: Rejoins,
    ) -> Self {
        Self {
            id,
            config,
            router,
            groups,
            nodes: OnceLock::new(),
            window: Mutex::new(None),
            takeovers: Takeovers::default(),
            rejoins,
        }
    }

    fn groups(&self) -> Result<&[Raft], Status> {
        let groups = self.groups.get().ok_or_else(|| Status::failed_precondition("the node is not set up"))?;
        Ok(groups)
    }

    fn nodes(&self) -> Result<&BTreeMap<u64, raftpb::Node>, Status> {
        self.nodes.get().ok_or_else(|| Status::failed_precondition("the node is not set up"))
    }

    /// Return the ids of the groups that this node initializes, and so leads first: node `id`
    /// takes the groups whose id leaves the remainder `id - 1` when divided by the node count.
    fn first_led_groups(&self) -> Result<Vec<u64>, Status> {
        let nodes = self.nodes()?.len() as u64;
        let groups = self.groups()?.len() as u64;
        Ok((0..groups).filter(|group_id| group_id % nodes == self.id - 1).collect())
    }
}

#[tonic::async_trait]
impl Admin for AdminService {
    async fn setup(&self, request: Request<pb::SetupRequest>) -> Result<Response<()>, Status> {
        let pb::SetupRequest { peers, groups } = request.into_inner();

        let nodes = peers.iter().map(|peer| {
            let node = raftpb::Node {
                node_id: peer.node_id,
                rpc_addr: peer.addr.clone(),
            };
            (peer.node_id, node)
        });
        let nodes = nodes.collect::<BTreeMap<_, _>>();
        if !nodes.contains_key(&self.id) {
            return Err(Status::invalid_argument(format!(
                "node {} is not among the peers",
                self.id
            )));
        }

        let others = peers.iter().filter(|peer| peer.node_id != self.id);
        let others = others.map(|peer| (peer.node_id, peer.addr.clone())).collect::<BTreeMap<_, _>>();
        self.router.connect(&others).map_err(internal)?;

        let mut rafts = Vec::new();
        for group_id in 0..groups {
            let network = GroupNetwork::new(self.router.clone(), group_id);
            let state_machine = Arc::new(StateMachineStore::default());
            let raft = Raft::new(
                self.id,
                self.config.clone(),
                network,
                LogStore::default(),
                state_machine,
            )
            .await;
            rafts.push(raft.map_err(internal)?);
        }

        let already = self.groups.set(rafts).is_err() || self.nodes.set(nodes).is_err();
        if already {
            return Err(Status::failed_precondition("the node is set up only once"));
        }
        Ok(Response::new(()))
    }

    async fn initialize(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        let groups = self.groups()?;
        let nodes = self.nodes()?;
        let led = self.first_led_groups()?;

        // A leader starts its heartbeat timer when it is elected. Spread the initializations over
        // one heartbeat interval, so that the leaders do not send their heartbeats in step.
        let interval = Duration::from_millis(self.config.heartbeat_interval);
        let started = Instant::now();
        for (i, group_id) in led.iter().enumerate() {
            let offset = interval.mul_f64(i as f64 / led.len() as f64);
            tokio::time::sleep_until(started + offset).await;

            let raft = &groups[*group_id as usize];
            raft.initialize(nodes.clone()).await.map_err(internal)?;
        }
        Ok(Response::new(()))
    }

    async fn wait_ready(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        let groups = self.groups()?;

        let waits = self.first_led_groups()?.into_iter().map(|group_id| {
            let raft = groups[group_id as usize].clone();
            async move {
                let msg = format!("group {group_id} elects this node and commits an entry");
                raft.wait(Some(READY_TIMEOUT)).metrics(is_ready_leader, msg).await
            }
        });
        try_join_all(waits).await.map_err(|e| Status::deadline_exceeded(e.to_string()))?;
        Ok(Response::new(()))
    }

    async fn begin_window(&self, request: Request<pb::BeginWindowRequest>) -> Result<Response<pb::Terms>, Status> {
        let down = request.into_inner().down.into_iter().collect::<BTreeSet<_>>();
        let groups = self.groups()?;

        let mut window = self.window.lock().expect("no thread panics while it holds the lock");
        if window.is_some() {
            return Err(Status::failed_precondition("a window is open"));
        }
        *window = Some(Window::begin(self.id, groups, self.router.stats(), down).map_err(internal)?);

        Ok(Response::new(terms(groups)))
    }

    async fn end_window(&self, _request: Request<()>) -> Result<Response<pb::WindowReport>, Status> {
        let groups = self.groups()?;

        let window = self.window.lock().expect("no thread panics while it holds the lock").take();
        let window = window.ok_or_else(|| Status::failed_precondition("no window is open"))?;
        let report = window.end(self.router.stats(), terms(groups)).map_err(internal)?;

        Ok(Response::new(report))
    }

    async fn get_leaders(&self, _request: Request<()>) -> Result<Response<pb::Leaders>, Status> {
        let groups = self.groups()?;

        let leaders = groups.iter().map(|raft| raft.server_metrics().borrow_watched().current_leader.unwrap_or(0));
        Ok(Response::new(pb::Leaders {
            leaders: leaders.collect(),
        }))
    }

    async fn get_terms(&self, _request: Request<()>) -> Result<Response<pb::Terms>, Status> {
        Ok(Response::new(terms(self.groups()?)))
    }

    async fn arm_failover(&self, request: Request<pb::ArmFailoverRequest>) -> Result<Response<()>, Status> {
        let group_ids = request.into_inner().group_ids;
        let groups = self.groups()?;

        self.takeovers.arm(groups, &group_ids, TAKEOVER_LIMIT).map_err(internal)?;
        Ok(Response::new(()))
    }

    async fn get_takeovers(&self, _request: Request<()>) -> Result<Response<pb::Takeovers>, Status> {
        Ok(Response::new(self.takeovers.recorded()))
    }

    async fn disarm_failover(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        self.takeovers.disarm();
        Ok(Response::new(()))
    }

    async fn arm_rejoin(&self, request: Request<pb::ArmRejoinRequest>) -> Result<Response<()>, Status> {
        let after_ns = request.into_inner().after_ns;
        let groups = self.groups()?;

        self.rejoins.arm(groups, after_ns).map_err(internal)?;
        Ok(Response::new(()))
    }

    async fn get_rejoins(&self, _request: Request<()>) -> Result<Response<pb::Rejoins>, Status> {
        Ok(Response::new(self.rejoins.recorded()))
    }

    async fn disarm_rejoin(&self, _request: Request<()>) -> Result<Response<()>, Status> {
        self.rejoins.disarm();
        Ok(Response::new(()))
    }
}

/// Return the current term of each of `groups`.
fn terms(groups: &[Raft]) -> pb::Terms {
    let terms = groups.iter().map(|raft| raft.metrics().borrow_watched().current_term);
    pb::Terms { terms: terms.collect() }
}

fn internal<E>(error: E) -> Status
where E: fmt::Display {
    Status::internal(error.to_string())
}
