use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use app_http::Client;
use openraft::Config;
use openraft::NodeInfo;
use openraft::SnapshotPolicy;
use openraft::async_runtime::AsyncRuntime;
use openraft::storage::RaftStateMachine;
use openraft::type_config::alias::AsyncRuntimeOf;
use raft_kv_rocksdb::Raft;
use raft_kv_rocksdb::RaftNode;
use raft_kv_rocksdb::StateMachineStore;
use raft_kv_rocksdb::TypeConfig;
use raft_kv_rocksdb::example_config;
use raft_kv_rocksdb::typ::ClientWriteResponse;
use raft_kv_rocksdb::typ::LogId;
use raft_kv_rocksdb::typ::Snapshot;
use raft_kv_rocksdb::typ::StoredMembership;
use tokio::sync::oneshot;

use crate::util;

pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A node whose runtime can be stopped while its databases stay available for restart.
pub struct Node {
    settings: Settings,
    raft: Option<Raft>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<anyhow::Result<()>>>,
}

impl Node {
    /// Start a node with automatic elections, heartbeats, and snapshots disabled.
    pub async fn new(id: u64, port_base: u16) -> anyhow::Result<Self> {
        util::init_observability();
        let dir = tempfile::tempdir()?.keep();
        tracing::info!(id, dir = %dir.display(), "node data directory; it stays when a test fails");
        let settings = Settings {
            id,
            dir,
            api_addr: util::api_addr(port_base, id),
            raft_addr: util::raft_addr(port_base, id),
        };
        let mut node = Self {
            settings,
            raft: None,
            stop: None,
            thread: None,
        };
        node.start().await?;
        Ok(node)
    }

    /// Access the running Raft instance.
    pub fn raft(&self) -> &Raft {
        self.raft.as_ref().expect("node is running")
    }

    /// Return the data directory retained across restarts and failed tests.
    pub fn dir(&self) -> &Path {
        &self.settings.dir
    }

    /// Return the Raft HTTP address.
    pub fn raft_addr(&self) -> &str {
        &self.settings.raft_addr
    }

    /// Initialize a single voter and wait for its initial logs to apply.
    pub async fn initialize(&self) -> anyhow::Result<()> {
        let node = NodeInfo::new(&self.settings.raft_addr, &self.settings.api_addr);
        let members = BTreeMap::from([(self.settings.id, node)]);
        self.raft().initialize(members).await?;
        self.wait_applied(1).await
    }

    /// Add another fixture node and wait for it to catch up through the real HTTP transport.
    pub async fn add_learner(&self, learner: &Self) -> anyhow::Result<()> {
        let node = NodeInfo::new(&learner.settings.raft_addr, &learner.settings.api_addr);
        let response = self.raft().add_learner(learner.settings.id, node, true).await?;
        let index = response.log_id.index();
        learner.wait_applied(index).await
    }

    /// Wait for an exact applied index with a finite deadline.
    pub async fn wait_applied(&self, index: u64) -> anyhow::Result<()> {
        let wait = self.raft().wait(Some(TIMEOUT));
        wait.applied_index(Some(index), "node applies expected log").await?;
        Ok(())
    }

    /// Commit a value and retain the complete response for replication assertions.
    pub async fn write(&self, key: &str, value: &str) -> anyhow::Result<ClientWriteResponse> {
        let request = types_kv::Request::set(key, value);
        let response = self.raft().client_write(request).await?;
        Ok(response)
    }

    /// Capture a checkpoint at the current applied index.
    pub async fn build_snapshot(&self) -> anyhow::Result<Snapshot> {
        let (last_applied, _) = self.applied_state().await?;
        let last_applied = last_applied.context("node has applied logs")?;
        let trigger = self.raft().trigger();
        trigger.snapshot().await?;
        let wait = self.raft().wait(Some(TIMEOUT));
        wait.snapshot(last_applied, "checkpoint is built").await?;
        let snapshot = self.raft().get_snapshot().await?;
        let snapshot = snapshot.context("checkpoint is available")?;
        Ok(snapshot)
    }

    /// Purge the logs covered by a checkpoint before exercising snapshot replication.
    pub async fn purge(&self, snapshot: &Snapshot) -> anyhow::Result<()> {
        let log_id = snapshot.meta.last_log_id.context("snapshot has a log id")?;
        let index = log_id.index();
        let trigger = self.raft().trigger();
        trigger.purge_log(index).await?;
        let wait = self.raft().wait(Some(TIMEOUT));
        wait.purged(Some(log_id), "snapshot logs are purged").await?;
        Ok(())
    }

    /// Read a complete versioned application response through the HTTP API.
    pub async fn read(&self, key: &str) -> anyhow::Result<types_kv::Response> {
        let client = Client::<TypeConfig>::new(self.settings.id, self.settings.api_addr.clone());
        let key = key.to_string();
        let response = client.read(&key).await?;
        Ok(response)
    }

    /// Read the state machine's durable applied log and membership together.
    pub async fn applied_state(&self) -> anyhow::Result<(Option<LogId>, StoredMembership)> {
        let state = self
            .raft()
            .with_state_machine(|sm: &mut StateMachineStore| Box::pin(async move { sm.applied_state().await }))
            .await?;
        let state = state?;
        Ok(state)
    }

    /// Read the persisted active generation name.
    pub fn current(&self) -> anyhow::Result<String> {
        let path = self.dir().join("sm/CURRENT");
        let current = fs::read_to_string(path)?;
        Ok(current)
    }

    /// List generation directories so failed and discarded transfers can be checked exactly.
    pub fn generations(&self) -> anyhow::Result<BTreeSet<PathBuf>> {
        let base_dir = self.dir().join("sm");
        let mut paths = BTreeSet::new();
        for entry in fs::read_dir(base_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                paths.insert(entry.path());
            }
        }
        Ok(paths)
    }

    /// Wait for receiving directories to be removed, checking the complete directory set.
    pub async fn wait_generations(&self, expected: &BTreeSet<PathBuf>) -> anyhow::Result<()> {
        let wait = async {
            loop {
                let actual = self.generations()?;
                if actual == *expected {
                    return Ok(());
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        };
        let result: anyhow::Result<()> = tokio::time::timeout(TIMEOUT, wait).await?;
        result?;
        Ok(())
    }

    /// Post a raw or fragmented snapshot body with a finite request deadline.
    pub async fn post_snapshot(&self, body: reqwest::Body) -> anyhow::Result<reqwest::Response> {
        let builder = reqwest::Client::builder();
        let builder = builder.no_proxy();
        let client = builder.build()?;
        let url = format!("http://{}/snapshot", self.raft_addr());
        let request = client.post(url);
        let request = request.body(body);
        let request = request.timeout(TIMEOUT);
        let response = request.send().await?;
        Ok(response)
    }

    /// Stop the whole runtime, including accepted HTTP connections and RocksDB handles.
    pub async fn stop(&mut self) -> anyhow::Result<()> {
        let Some(stop) = self.stop.take() else {
            return Ok(());
        };
        self.raft = None;
        let sent = stop.send(());
        self.join_thread().await?;
        sent.map_err(|()| anyhow::anyhow!("node stopped before shutdown was requested"))?;
        Ok(())
    }

    /// Reopen the same databases and HTTP endpoints on a fresh runtime.
    pub async fn restart(&mut self) -> anyhow::Result<()> {
        self.stop().await?;
        self.start().await
    }

    /// Remove test data after every assertion has succeeded.
    pub async fn cleanup(mut self) -> anyhow::Result<()> {
        self.stop().await?;
        fs::remove_dir_all(self.dir())?;
        Ok(())
    }

    async fn start(&mut self) -> anyhow::Result<()> {
        let settings = self.settings.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        self.thread = Some(thread::spawn(move || {
            let mut runtime = AsyncRuntimeOf::<TypeConfig>::new(2);
            runtime.block_on(settings.run(ready_tx, stop_rx))
        }));
        self.stop = Some(stop_tx);
        let ready = tokio::time::timeout(TIMEOUT, ready_rx).await?;
        match ready {
            Ok(raft) => self.raft = Some(raft),
            Err(error) => {
                self.join_thread().await?;
                return Err(error.into());
            }
        }
        self.wait_ready().await
    }

    async fn join_thread(&mut self) -> anyhow::Result<()> {
        let thread = self.thread.take().context("node has a runtime thread")?;
        let joined = tokio::task::spawn_blocking(move || thread.join()).await?;
        let result = joined.map_err(|_| anyhow::anyhow!("node runtime panicked"))?;
        result?;
        Ok(())
    }

    async fn wait_ready(&self) -> anyhow::Result<()> {
        for addr in [&self.settings.api_addr, &self.settings.raft_addr] {
            self.wait_listening(addr).await?;
        }
        Ok(())
    }

    async fn wait_listening(&self, addr: &str) -> anyhow::Result<()> {
        let wait = async {
            loop {
                match tokio::net::TcpStream::connect(addr).await {
                    Ok(connection) => {
                        drop(connection);
                        return Ok(());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                        tokio::time::sleep(POLL_INTERVAL).await;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        };
        let result: anyhow::Result<()> = tokio::time::timeout(TIMEOUT, wait).await?;
        result?;
        Ok(())
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.raft = None;
        if let Some(stop) = self.stop.take() {
            let sent = stop.send(());
            if sent.is_err() {
                tracing::error!(id = self.settings.id, "node stopped before fixture shutdown");
            }
        }
        let Some(thread) = self.thread.take() else {
            return;
        };
        match thread.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "node failed during fixture shutdown"),
            Err(_) => tracing::error!("node runtime panicked during fixture shutdown"),
        }
    }
}

#[derive(Clone)]
struct Settings {
    id: u64,
    dir: PathBuf,
    api_addr: String,
    raft_addr: String,
}

impl Settings {
    /// Assemble the production node, hand its Raft handle to the test, then serve until `stop`.
    async fn run(self, ready: oneshot::Sender<Raft>, stop: oneshot::Receiver<()>) -> anyhow::Result<()> {
        let config = Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            snapshot_policy: SnapshotPolicy::Never,
            max_in_snapshot_log_to_keep: 0,
            ..example_config()
        };
        let node = RaftNode::new(self.id, &self.dir, self.api_addr, self.raft_addr, config).await?;
        let raft = node.app.raft.clone();
        ready.send(raft.clone()).map_err(|_| anyhow::anyhow!("test stopped before node became ready"))?;
        let result = tokio::select! {
            result = node.run() => result,
            result = stop => result.map_err(std::io::Error::other),
        };
        raft.shutdown().await?;
        result?;
        Ok(())
    }
}
