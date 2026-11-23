use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use app_http::Client;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ClientWriteError;
use openraft::type_config::TypeConfigExt;
use tokio::task::JoinHandle;
use types_kv::DedupRequest;
use types_kv::Request;
use types_kv::RequestId;
use types_kv::Response;

use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

/// A node of the test cluster. While `block_append` is set, its Raft server leaves every
/// `/append` request unanswered, without handling it.
struct TestNode {
    app: Arc<App>,
    servers: Vec<JoinHandle<std::io::Result<()>>>,
    block_append: Arc<AtomicBool>,
}

async fn node(id: u64, base: u16) -> TestNode {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        election_timeout_min: 100,
        election_timeout_max: 200,
        heartbeat_interval: 50,
        ..example_config()
    };
    let (raft, data) = new_raft_node(id, config).await;
    let api_addr = format!("127.0.0.1:{}", base + id as u16);
    let raft_addr = format!("127.0.0.1:{}", base + 100 + id as u16);
    let app = Arc::new(App {
        id,
        api_addr: api_addr.clone(),
        raft_addr: raft_addr.clone(),
        raft,
        data,
    });
    let block_append = Arc::new(AtomicBool::new(false));
    let gate = block_append.clone();
    let raft_server =
        app_http::Server::new(app.clone())
            .post(
                "/append",
                move |app: Arc<App>, request: openraft::raft::AppendEntriesRequest<TypeConfig>| {
                    let gate = gate.clone();
                    async move {
                        if gate.load(Ordering::Relaxed) {
                            std::future::pending::<()>().await;
                        }
                        app.raft.append_entries(request).await.map(network_v2_http::AppendReply::from)
                    }
                },
            )
            .post(
                "/vote",
                |app: Arc<App>, request: openraft::raft::VoteRequest<TypeConfig>| async move {
                    app.raft.vote(request).await
                },
            );
    let rpc = tokio::spawn(raft_server.run(raft_addr.clone()));
    let http = tokio::spawn(new_http_server(app.clone()).run(api_addr.clone()));
    for addr in [api_addr, raft_addr] {
        let ready = async {
            while tokio::net::TcpStream::connect(&addr).await.is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(TIMEOUT, ready).await.unwrap();
    }
    TestNode {
        app,
        servers: vec![rpc, http],
        block_append,
    }
}

/// Start voters 1, 2 and 3 led by node 1, and return them once each has applied the membership.
async fn cluster(base: u16) -> Vec<TestNode> {
    let mut nodes = Vec::new();
    for id in 1..=3 {
        nodes.push(node(id, base).await);
    }
    let leader = &nodes[0].app;
    let info = Node::new(&leader.raft_addr, &leader.api_addr);
    leader.raft.initialize(BTreeMap::from([(1, info)])).await.unwrap();
    leader.raft.wait(Some(TIMEOUT)).applied_index(Some(1), "initial leader applies").await.unwrap();
    for peer in &nodes[1..] {
        let info = Node::new(&peer.app.raft_addr, &peer.app.api_addr);
        leader.raft.add_learner(peer.app.id, info, true).await.unwrap();
    }
    let changed = leader.raft.change_membership(BTreeSet::from([1, 2, 3]), false).await.unwrap();
    for node in &nodes {
        let index = Some(changed.log_id.index());
        node.app.raft.wait(Some(TIMEOUT)).applied_index(index, "membership applied").await.unwrap();
    }
    nodes
}

async fn shutdown(nodes: Vec<TestNode>) {
    for node in nodes {
        node.app.raft.shutdown().await.unwrap();
        for server in node.servers {
            server.abort();
        }
    }
}

#[test]
fn a_write_resent_after_its_response_was_lost_gets_its_first_outcome() {
    TypeConfig::run(async {
        let nodes = cluster(40000).await;
        let leader = &nodes[0].app;
        let client = Client::<TypeConfig>::new(1, leader.api_addr.clone());
        let write = DedupRequest::new(RequestId::new(7, 1), Request::set("key", "first"));

        tracing::info!(
            "Leader 1 applies write {}, and its response never reaches the client",
            write.id
        );
        let first = leader.raft.client_write(write.clone()).await.unwrap();

        tracing::info!(
            "The client resends write {}: a second entry commits and gets the first outcome",
            write.id
        );
        {
            let resent = client.write_with_retries(&write).await.unwrap().unwrap();
            assert_eq!(first.log_id.index() + 1, resent.log_id.index());
            let expected = Response::new("first", first.log_id.index());
            assert_eq!(expected, first.data);
            assert_eq!(expected, resent.data);
            assert_eq!(expected.value, leader.data.get("key").await);
        }

        tracing::info!("The next write of the client, with serial 2, applies");
        {
            let next = DedupRequest::new(RequestId::new(7, 2), Request::set("key", "second"));
            let applied = client.write_with_retries(&next).await.unwrap().unwrap();
            let expected = Response::new("second", applied.log_id.index());
            assert_eq!(expected, applied.data);
            assert_eq!(expected.value, leader.data.get("key").await);
        }
        shutdown(nodes).await;
    });
}

#[test]
fn a_discarded_write_applies_once_when_resent() {
    TypeConfig::run(async {
        let nodes = cluster(41000).await;
        let (node1, node2, node3) = (&nodes[0], &nodes[1], &nodes[2]);
        let write = DedupRequest::new(RequestId::new(7, 1), Request::set("key", "value"));
        let index = node1.app.raft.metrics().borrow_watched().last_log_index.unwrap() + 1;

        tracing::info!(
            "Leave node 1's appends to nodes 2 and 3 unanswered, so write {} reaches only node 1, at index {}",
            write.id,
            index
        );
        let pending = {
            node2.block_append.store(true, Ordering::Relaxed);
            node3.block_append.store(true, Ordering::Relaxed);
            let client = Client::<TypeConfig>::new(1, node1.app.api_addr.clone());
            let request = write.clone();
            let pending = tokio::spawn(async move {
                let result = client.write(&request).await;
                (result, client)
            });
            node1.app.raft.wait(Some(TIMEOUT)).log_index(Some(index), "node 1 appends the write").await.unwrap();
            pending
        };

        tracing::info!("Elect node 2 with the vote of node 3; neither holds the write");
        {
            // Let node 3's lease of leader 1 expire: no append of node 1 reaches it.
            tokio::time::sleep(Duration::from_millis(250)).await;
            node2.app.raft.trigger().elect(false).await.unwrap();
            node2.app.raft.wait(Some(TIMEOUT)).state(ServerState::Leader, "node 2 elected").await.unwrap();
        }

        tracing::info!(
            "Node 2's no-op replaces the write at index {} in node 1's log, so node 1 answers LogEntryDiscarded",
            index
        );
        let client = {
            node3.block_append.store(false, Ordering::Relaxed);
            let (result, client) = tokio::time::timeout(TIMEOUT, pending).await.unwrap().unwrap();
            let error = result.unwrap().unwrap_err();
            let ClientWriteError::LogEntryDiscarded(forward) = error else {
                panic!("expect LogEntryDiscarded, got: {error}");
            };
            assert_eq!(Some(2), forward.leader_id);
            client
        };

        tracing::info!("The client resends write {} to node 2, which applies it once", write.id);
        let expected = {
            let noop = Some(index);
            node2.app.raft.wait(Some(TIMEOUT)).applied_index(noop, "node 2 applies its no-op").await.unwrap();
            let applied = client.write_with_retries(&write).await.unwrap().unwrap();
            let expected = Response::new("value", applied.log_id.index());
            assert_eq!(expected, applied.data);
            assert_eq!(expected.value, node2.app.data.get("key").await);
            expected
        };

        tracing::info!(
            "Resending write {} once more gets the same outcome without applying it again",
            write.id
        );
        {
            let resent = client.write_with_retries(&write).await.unwrap().unwrap();
            assert_eq!(expected, resent.data);
            assert_eq!(expected.value, node2.app.data.get("key").await);
        }
        shutdown(nodes).await;
    });
}
