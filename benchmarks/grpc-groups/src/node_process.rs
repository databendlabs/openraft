use std::io::BufRead;
use std::io::BufReader;
use std::net::SocketAddr;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use tonic::transport::Channel;

use crate::batching::Batching;
use crate::protobuf::admin_client::AdminClient;

/// A node process that the driver started, with a client of its admin service.
pub(crate) struct NodeProcess {
    pub(crate) id: u64,
    pub(crate) addr: SocketAddr,
    child: Child,
    admin: AdminClient<Channel>,
}

impl NodeProcess {
    /// Start node `id`, with `workers` tokio threads, the heartbeat options `batching` and the
    /// openraft `Config` flags `raft_args`, and connect to its admin service.
    pub(crate) async fn start(
        id: u64,
        workers: usize,
        batching: Batching,
        raft_args: &[String],
    ) -> anyhow::Result<Self> {
        let mut command = Command::new(std::env::current_exe()?);
        command.args(["node", "--id", &id.to_string(), "--workers", &workers.to_string()]);
        command.args(batching.flags());
        command.arg("--");
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

    /// Stop the node with `SIGSTOP`, as a hang would: it keeps its connections and its state, but
    /// runs nothing, its timers included, until [`Self::resume()`].
    pub(crate) fn stop(&self) -> anyhow::Result<()> {
        self.signal(libc::SIGSTOP).context("stop the node with SIGSTOP")
    }

    /// Resume the node with `SIGCONT` after [`Self::stop()`].
    pub(crate) fn resume(&self) -> anyhow::Result<()> {
        self.signal(libc::SIGCONT).context("resume the node with SIGCONT")
    }

    /// Cut the node off from the other nodes, as a blackholed network would: it keeps running,
    /// but no Raft request crosses the cut until [`Self::heal()`]. The admin calls still do.
    pub(crate) async fn cut(&self) -> anyhow::Result<()> {
        self.admin().cut(()).await.context("cut the links of the node")?;
        Ok(())
    }

    /// Heal the links of the node after [`Self::cut()`].
    pub(crate) async fn heal(&self) -> anyhow::Result<()> {
        self.admin().heal(()).await.context("heal the links of the node")?;
        Ok(())
    }

    fn signal(&self, signal: libc::c_int) -> std::io::Result<()> {
        let pid = libc::pid_t::try_from(self.child.id()).map_err(std::io::Error::other)?;
        // SAFETY: `kill()` reads only its arguments, and `pid` is a child of this process that it
        // has not waited for, so the pid still names that child.
        let ret = unsafe { libc::kill(pid, signal) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
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
