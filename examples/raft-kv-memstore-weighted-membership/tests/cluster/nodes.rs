//! Run each node of a test cluster as a process of this example's binary, so that a test can stop
//! a node by killing its process.

use std::collections::BTreeMap;
use std::fs::File;
use std::process::Child;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use app_http::Client;
use raft_kv_memstore_weighted_membership::TypeConfig;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// API ports are `PORT_BASE + id` and Raft ports `PORT_BASE + 1000 + id`, apart from the ports
/// that the tests of the other examples use.
const PORT_BASE: u16 = 36000;

/// How long a test waits for a node to start or to reach an expected state.
const TIMEOUT: Duration = Duration::from_secs(10);

pub fn api_addr(id: u64) -> String {
    format!("127.0.0.1:{}", PORT_BASE + id as u16)
}

pub fn raft_addr(id: u64) -> String {
    format!("127.0.0.1:{}", PORT_BASE + 1000 + id as u16)
}

/// The running node processes. Dropping it kills every process, also when a test panics.
pub struct Nodes {
    processes: BTreeMap<u64, Child>,
}

impl Nodes {
    /// Starts a process of `raft-key-value-weighted` for each of `ids`, and waits until each
    /// serves its API. A node writes its log to `node-<id>.log` in `CARGO_TARGET_TMPDIR`.
    pub async fn start(ids: impl IntoIterator<Item = u64>) -> anyhow::Result<Self> {
        let mut nodes = Nodes {
            processes: BTreeMap::new(),
        };

        for id in ids {
            let log = File::create(format!("{}/node-{}.log", env!("CARGO_TARGET_TMPDIR"), id))?;
            let process = Command::new(env!("CARGO_BIN_EXE_raft-key-value-weighted"))
                .args(["--id", &id.to_string()])
                .args(["--api-addr", &api_addr(id)])
                .args(["--raft-addr", &raft_addr(id)])
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?;
            nodes.processes.insert(id, process);
        }

        for (id, process) in nodes.processes.iter_mut() {
            let client = Client::<TypeConfig>::new(*id, api_addr(*id));
            let what = format!("node {} starts", id);
            wait(&what, async || {
                if let Some(status) = process.try_wait()? {
                    anyhow::bail!("node {} exited with {}", id, status);
                }
                Ok(client.metrics().await.is_ok())
            })
            .await?;
        }

        Ok(nodes)
    }

    /// Stops node `id` by killing its process, which loses its in-memory log and state machine.
    pub fn kill(&mut self, id: u64) -> anyhow::Result<()> {
        let mut process = self.processes.remove(&id).with_context(|| format!("node {} is not running", id))?;
        process.kill()?;
        process.wait()?;
        Ok(())
    }
}

impl Drop for Nodes {
    fn drop(&mut self) {
        for process in self.processes.values_mut() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

/// Sends `req` as JSON to `path` of node `id`, and decodes the JSON response.
///
/// The membership routes of this example take bodies that `app_http::Client` does not send.
pub async fn post<Req, Resp>(id: u64, path: &str, req: &Req) -> anyhow::Result<Resp>
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    let http = reqwest::Client::builder().no_proxy().build()?;
    let url = format!("http://{}/{}", api_addr(id), path);
    let resp = http.post(url).json(req).send().await?;
    Ok(resp.json().await?)
}

/// Polls `check` until it returns `true`, for at most [`TIMEOUT`].
pub async fn wait(what: &str, mut check: impl AsyncFnMut() -> anyhow::Result<bool>) -> anyhow::Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    while !check().await? {
        anyhow::ensure!(Instant::now() < deadline, "timeout after {:?}: {}", TIMEOUT, what);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
