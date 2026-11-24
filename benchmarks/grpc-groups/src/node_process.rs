use std::io::BufRead;
use std::io::BufReader;
use std::net::SocketAddr;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use tonic::transport::Channel;

use crate::protobuf::admin_client::AdminClient;

/// A node process that the driver started, with a client of its admin service.
pub(crate) struct NodeProcess {
    pub(crate) id: u64,
    pub(crate) addr: SocketAddr,
    child: Child,
    admin: AdminClient<Channel>,
}

impl NodeProcess {
    /// Start node `id`, with `workers` tokio threads and the openraft `Config` flags
    /// `raft_args`, and connect to its admin service.
    pub(crate) async fn start(id: u64, workers: usize, raft_args: &[String]) -> anyhow::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command.args(["node", "--id", &id.to_string(), "--workers", &workers.to_string(), "--"]);
        command.args(raft_args);
        // The node exits once its stdin closes, which happens when this process exits.
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = command.spawn().with_context(|| format!("start node {id}"))?;

        let stdout = child.stdout.take().context("take the stdout of the node")?;
        let read_line = move || {
            let mut line = String::new();
            BufReader::new(stdout).read_line(&mut line).map(|_| line)
        };
        let line = tokio::task::spawn_blocking(read_line).await??;
        let addr = line.trim().parse::<SocketAddr>();
        let addr = addr.with_context(|| format!("node {id} printed no address: {line:?}"))?;

        let admin = AdminClient::connect(format!("http://{addr}")).await?;
        Ok(Self { id, addr, child, admin })
    }

    /// Return a client of the admin service of the node.
    pub(crate) fn admin(&self) -> AdminClient<Channel> {
        self.admin.clone()
    }

    /// Kill the node with `SIGKILL`, which it cannot catch, as a crash would stop it.
    pub(crate) fn crash(mut self) -> anyhow::Result<()> {
        self.child.kill()?;
        self.child.wait()?;
        Ok(())
    }
}

impl Drop for NodeProcess {
    fn drop(&mut self) {
        // Stop the node without shutting its groups down: with thousands of groups that takes
        // longer than a run, and it measures nothing. A node that has already stopped makes this
        // fail, which needs no handling.
        self.child.kill().ok();
        self.child.wait().ok();
    }
}
