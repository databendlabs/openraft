//! EzRaft KV Store Example
//!
//! A simple distributed key-value store built on EzRaft.
//! Run multiple instances to form a cluster:
//!
//! ```bash
//! # Terminal 1 (first node - creates cluster)
//! cargo run --example kvstore -- --addr 127.0.0.1:8080
//!
//! # Terminal 2 (joins via seed node)
//! cargo run --example kvstore -- --addr 127.0.0.1:8081 --seed 127.0.0.1:8080
//!
//! # Terminal 3 (joins via seed node)
//! cargo run --example kvstore -- --addr 127.0.0.1:8082 --seed 127.0.0.1:8080
//! ```
//!
//! Then drive it through the write API, which takes the `Request` type below as JSON,
//! and read a key back directly - reads never need the log:
//!
//! ```bash
//! curl -X POST 127.0.0.1:8080/api/write -H 'Content-Type: application/json' \
//!     -d '{"Set": {"key": "hello", "value": "world"}}'
//!
//! curl '127.0.0.1:8080/api/read?key=hello'
//! ```

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use clap::Parser;
use ezraft::EzApp;
use ezraft::EzConfig;
use ezraft::EzRaft;
use ezraft::FileStorage;
use serde::Deserialize;
use serde::Serialize;
use tracing_subscriber::EnvFilter;

// Define application request types
//
// Reads are deliberately not requests: they are served from local state via
// `GET /api/read` (or `EzRaft::read` in code) with no consensus round and no log
// entry. The alternative - a `Get` variant applied through the log - is worth its
// cost only when a read must be linearizable from any node, not just the leader.
#[derive(Serialize, Deserialize, Debug, Clone, derive_more::Display)]
pub enum Request {
    #[display("Set({key})")]
    Set { key: String, value: String },
    #[display("Delete({key})")]
    Delete { key: String },
}

// Define application response type
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Response {
    pub value: Option<String>,
}

// The application: its state plus one method of business logic.
// Snapshots are derived from the state via serde, hence the serde derives.
#[derive(Default, Serialize, Deserialize)]
struct KvApp {
    data: BTreeMap<String, String>,
}

#[async_trait::async_trait]
impl EzApp for KvApp {
    type Request = Request;
    type Response = Response;

    async fn apply(&mut self, req: Request) -> Response {
        match req {
            Request::Set { key, value } => {
                // The replaced value, if any: a caller that overwrites a key learns what was
                // there without a second round trip - and, unlike a read it issues itself, the
                // answer is the one this very entry replaced.
                let value = self.data.insert(key, value);
                Response { value }
            }
            Request::Delete { key } => {
                let value = self.data.remove(&key);
                Response { value }
            }
        }
    }

    // Serves `GET /api/read?key=...` straight from the map - an indexed lookup,
    // not a scan of the serialized state.
    fn read(&self, key: &str) -> Option<serde_json::Value> {
        self.data.get(key).map(|value| serde_json::Value::String(value.clone()))
    }
}

/// Command-line arguments for the KV store
#[derive(clap::Parser)]
struct Args {
    /// HTTP bind address (e.g., "127.0.0.1:8080")
    #[arg(long, default_value = "127.0.0.1:8080")]
    addr: String,

    /// Seed node address to join existing cluster
    #[arg(long)]
    seed: Option<String>,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // Without this, everything Raft reports about a cluster that is not working goes nowhere.
    // Warnings and errors by default; set RUST_LOG=info (or debug) to follow what Raft is doing.
    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .init();

    let args = Args::parse();
    let addr = args.addr;
    let seed = args.seed;

    // Create app and storage (use addr for directory name)
    let base_dir = PathBuf::from(format!("./data/{}", addr.replace(':', "-")));

    // The bundled storage: readable JSON files, no fsync. A deployment implements
    // `EzStorage` itself - see the caveats on `FileStorage`.
    let app = KvApp::default();
    let storage = FileStorage::new(base_dir).await?;

    // Create EzRaft instance: the first node starts the cluster, the rest join it
    let config = EzConfig::default();
    let raft = match &seed {
        Some(seed) => EzRaft::join(&addr, seed, app, storage, config).await?,
        None => EzRaft::create(&addr, app, storage, config).await?,
    };

    println!("Node {} listening on {}", raft.node_id(), addr);
    print_next_steps(&addr, seed.is_some());

    // Start HTTP server
    raft.serve().await?;

    Ok(())
}

/// Print the commands that drive the node just started
///
/// A node that announces only that it is listening leaves the reader with nothing to type: the
/// body `/api/write` takes is this file's `Request` enum as JSON, which cannot be guessed from
/// outside the source.
fn print_next_steps(addr: &str, seed_given: bool) {
    // Without a seed this node forms a cluster of its own, and every other node must be pointed
    // at it. A reader who starts a second node without `--seed` gets two separate clusters that
    // never merge, so the invitation to join is printed with the address already filled in.
    if !seed_given && let Some(next) = next_addr(addr) {
        println!(
            "\nNo --seed given, so this node is the cluster's founding member. \
             Every other node joins through it - from another terminal:\n    \
             cargo run -p ezraft --example kvstore -- --addr {next} --seed {addr}"
        );
    }

    println!(
        r#"
Write a key - any node accepts one, a follower forwards it to the leader:
    curl -X POST {addr}/api/write -H 'Content-Type: application/json' \
        -d '{{"Set": {{"key": "hello", "value": "world"}}}}'

Read it back - answered from this node's memory, no consensus round:
    curl '{addr}/api/read?key=hello'

Cluster state - leader, term, log index, membership:
    curl {addr}/api/metrics
"#
    );
}

/// The address one port up, to suggest where the next node could listen
fn next_addr(addr: &str) -> Option<String> {
    let (host, port) = addr.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?.checked_add(1)?;
    Some(format!("{}:{}", host, port))
}
