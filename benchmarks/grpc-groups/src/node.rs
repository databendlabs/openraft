use std::io::Read;
use std::io::Write;
use std::sync::Arc;
use std::sync::OnceLock;

use openraft::Config;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;

use crate::admin_service::AdminService;
use crate::batching::Batching;
use crate::protobuf::admin_server::AdminServer;
use crate::protobuf::group_raft_server::GroupRaftServer;
use crate::raft_config;
use crate::raft_service::RaftService;
use crate::rejoins::Rejoins;
use crate::router::Router;

#[derive(Debug, clap::Args)]
pub(crate) struct NodeArgs {
    /// The id of this node, from 1 up.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    id: u64,

    /// The worker threads of the tokio runtime.
    #[arg(long, default_value_t = 4)]
    workers: usize,

    #[command(flatten)]
    batching: Batching,

    /// openraft `Config` flags, after `--`.
    #[arg(last = true)]
    raft_args: Vec<String>,
}

/// Serve the Raft requests of the peer nodes and the requests of the driver, until the driver
/// closes the stdin of this process.
pub(crate) fn run(args: NodeArgs) -> anyhow::Result<()> {
    let mut config = Arc::unwrap_or_clone(raft_config::parse(&args.raft_args)?);
    if args.batching.node_heartbeat_timer {
        // The node's timer triggers the heartbeats of all groups instead.
        config.enable_heartbeat = false;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(args.workers).enable_all().build()?;

    exit_when_stdin_closes();
    runtime.block_on(serve(args.id, Arc::new(config), args.batching))
}

async fn serve(id: u64, config: Arc<Config>, batching: Batching) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;

    // The driver reads the address of this node from the first line of its stdout.
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{}", listener.local_addr()?)?;
    stdout.flush()?;

    let groups = Arc::new(OnceLock::new());
    let rejoins = Rejoins::default();
    let router = Router::new(batching.batch_heartbeats);
    let raft_service = RaftService::new(groups.clone(), rejoins.clone(), router.partition().clone());
    let admin_service = AdminService::new(id, config, batching, router, groups, rejoins);

    // A heartbeat and its reply take a few bytes each. Send them at once, rather than waiting for
    // more data as Nagle's algorithm would.
    let incoming = TcpIncoming::from(listener).with_nodelay(Some(true));
    Server::builder()
        .add_service(GroupRaftServer::new(raft_service))
        .add_service(AdminServer::new(admin_service))
        .serve_with_incoming(incoming)
        .await?;
    Ok(())
}

/// Exit once stdin closes. The driver holds the other end, so the nodes stop whenever the driver
/// does, even when it is killed.
fn exit_when_stdin_closes() {
    std::thread::spawn(|| {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 64];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => std::process::exit(0),
                Ok(_) => {}
            }
        }
    });
}
