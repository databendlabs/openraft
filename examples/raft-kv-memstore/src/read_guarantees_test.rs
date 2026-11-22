use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use app_http::Client;
use app_http::FollowerReadError;
use app_http::Readiness;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::impls::ProgressResponder;
use openraft::type_config::TypeConfigExt;
use tokio::task::JoinHandle;

use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

type TestNode = (
    Arc<App>,
    JoinHandle<std::io::Result<()>>,
    JoinHandle<std::io::Result<()>>,
    Arc<AtomicBool>,
);

async fn node(
    id: u64,
    base: u16,
) -> (
    Arc<App>,
    JoinHandle<std::io::Result<()>>,
    JoinHandle<std::io::Result<()>>,
    Arc<AtomicBool>,
) {
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
    let blocked = Arc::new(AtomicBool::new(false));
    let append_gate = blocked.clone();
    let vote_gate = blocked.clone();
    let server = app_http::Server::new(app.clone())
        .post(
            "/append",
            move |app: Arc<App>, request: openraft::raft::AppendEntriesRequest<TypeConfig>| {
                let gate = append_gate.clone();
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
            move |app: Arc<App>, request: openraft::raft::VoteRequest<TypeConfig>| {
                let gate = vote_gate.clone();
                async move {
                    if gate.load(Ordering::Relaxed) {
                        std::future::pending::<()>().await;
                    }
                    app.raft.vote(request).await
                }
            },
        );
    let rpc = tokio::spawn(server.run(raft_addr.clone()));
    let http = tokio::spawn(new_http_server(app.clone()).run(api_addr.clone()));
    for addr in [api_addr, raft_addr] {
        let ready = async {
            while tokio::net::TcpStream::connect(&addr).await.is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(TIMEOUT, ready).await.unwrap();
    }
    (app, rpc, http, blocked)
}

/// Start voters 1, 2 and 3 led by node 1, and return them once every node has applied `key=old`.
async fn cluster(base: u16) -> (Vec<TestNode>, typ::ClientWriteResponse) {
    let mut nodes = Vec::new();
    for id in 1..=3 {
        nodes.push(node(id, base).await);
    }
    let leader = &nodes[0].0;
    let info = Node::new(&leader.raft_addr, &leader.api_addr);
    leader.raft.initialize(BTreeMap::from([(1, info)])).await.unwrap();
    leader.raft.wait(Some(TIMEOUT)).applied_index(Some(1), "initial leader applies").await.unwrap();
    for (peer, ..) in &nodes[1..] {
        let info = Node::new(&peer.raft_addr, &peer.api_addr);
        leader.raft.add_learner(peer.id, info, true).await.unwrap();
    }
    leader.raft.change_membership(BTreeSet::from([1, 2, 3]), false).await.unwrap();
    let written = leader.raft.client_write(types_kv::Request::set("key", "old")).await.unwrap();
    for (app, ..) in &nodes {
        let index = Some(written.log_id.index());
        app.raft.wait(Some(TIMEOUT)).applied_index(index, "key=old applied").await.unwrap();
    }
    (nodes, written)
}

/// Block the state machine worker of `app` until the returned sender sends or is dropped.
async fn pause_apply(app: &App) -> tokio::sync::oneshot::Sender<()> {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    app.raft
        .external_state_machine_request(move |_| {
            Box::pin(async move {
                entered_tx.send(()).unwrap();
                let _released = release_rx.await;
            })
        })
        .await
        .unwrap();
    tokio::time::timeout(TIMEOUT, entered_rx).await.unwrap().unwrap();
    release_tx
}

async fn shutdown(nodes: Vec<TestNode>) {
    for (app, rpc, http, _) in nodes {
        app.raft.shutdown().await.unwrap();
        rpc.abort();
        http.abort();
    }
}

#[test]
fn isolated_former_leader_serves_only_stale_local_reads() {
    TypeConfig::run(async {
        let (old, rpc_old, http_old, block_old) = node(1, 48000).await;
        let (next, rpc_next, http_next, _) = node(2, 48000).await;
        let (third, rpc_third, http_third, _) = node(3, 48000).await;
        let info = Node::new(&old.raft_addr, &old.api_addr);
        old.raft.initialize(BTreeMap::from([(1, info)])).await.unwrap();
        old.raft.wait(Some(TIMEOUT)).applied_index(Some(1), "initial leader applies").await.unwrap();
        for peer in [&next, &third] {
            let info = Node::new(&peer.raft_addr, &peer.api_addr);
            old.raft.add_learner(peer.id, info, true).await.unwrap();
        }
        old.raft.change_membership(BTreeSet::from([1, 2, 3]), false).await.unwrap();
        let written = old.raft.client_write(types_kv::Request::set("key", "old")).await.unwrap();
        for peer in [&next, &third] {
            peer.raft
                .wait(Some(TIMEOUT))
                .applied_index(Some(written.log_id.index()), "old value replicated")
                .await
                .unwrap();
        }

        tracing::info!("Cut inbound Raft traffic to the old leader and elect a replacement");
        {
            block_old.store(true, Ordering::Relaxed);
            // Let the old leadership lease expire before the explicit election.
            tokio::time::sleep(Duration::from_millis(250)).await;
            next.raft.trigger().elect(false).await.unwrap();
            next.raft.wait(Some(TIMEOUT)).state(ServerState::Leader, "replacement elected").await.unwrap();
            let index = written.log_id.index() + 1;
            next.raft
                .wait(Some(TIMEOUT))
                .applied_index(Some(index), "replacement commits its no-op")
                .await
                .unwrap();
        }
        let newer = next.raft.client_write(types_kv::Request::set("key", "new")).await.unwrap();
        let old_metrics = old.raft.metrics();
        let old_state = old_metrics.borrow_watched().state;
        assert_eq!(ServerState::Leader, old_state);
        let old_client = Client::<TypeConfig>::new(1, old.api_addr.clone());
        let key = "key".to_string();
        tracing::info!(
            "Local reads and readiness expose the old leadership, while the linearizable endpoint rejects it"
        );
        {
            let local = old_client.read(&key).await.unwrap();
            assert_eq!(written.data, local);
            let readiness = Readiness {
                leader: Some(1),
                is_leader: true,
                applied_leader_noop: true,
                last_applied: Some(written.log_id),
            };
            assert_eq!(readiness, old_client.readiness().await.unwrap());
            let result = tokio::time::timeout(TIMEOUT, old_client.linearizable_read(&key)).await.unwrap().unwrap();
            let error = result.unwrap_err();
            let rejected = matches!(
                error,
                openraft::errors::LinearizableReadError::ForwardToLeader(_)
                    | openraft::errors::LinearizableReadError::QuorumNotEnough(_)
            );
            assert!(rejected);
            let client = Client::<TypeConfig>::new(2, next.api_addr.clone());
            let current = client.linearizable_read(&key).await.unwrap().unwrap();
            assert_eq!(newer.data, current);
        }
        for app in [old, next, third] {
            app.raft.shutdown().await.unwrap();
        }
        for server in [rpc_old, http_old, rpc_next, http_next, rpc_third, http_third] {
            server.abort();
        }
    });
}

#[test]
fn linearizable_endpoint_waits_for_committed_application_effects() {
    TypeConfig::run(async {
        let (app, rpc, http, _) = node(1, 49000).await;
        let info = Node::new(&app.raft_addr, &app.api_addr);
        app.raft.initialize(BTreeMap::from([(1, info)])).await.unwrap();
        app.raft.wait(Some(TIMEOUT)).applied_index(Some(1), "initial leader applies").await.unwrap();
        let written = app.raft.client_write(types_kv::Request::set("key", "old")).await.unwrap();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        tracing::info!("Pause application while allowing a new write to commit");
        {
            app.raft
                .external_state_machine_request(move |_| {
                    Box::pin(async move {
                        entered_tx.send(()).unwrap();
                        let _released = release_rx.await;
                    })
                })
                .await
                .unwrap();
            tokio::time::timeout(TIMEOUT, entered_rx).await.unwrap().unwrap();
        }
        let (responder, committed, completed) = ProgressResponder::new();
        app.raft.write(types_kv::Request::set("key", "new")).responder(responder).await.unwrap();
        let committed = tokio::time::timeout(TIMEOUT, committed).await.unwrap().unwrap();
        let client = Client::<TypeConfig>::new(1, app.api_addr.clone());
        let key = "key".to_string();
        let local = client.read(&key).await.unwrap();
        assert_eq!(written.data, local);
        tracing::info!("The linearizable endpoint remains pending until application resumes");
        {
            let mut read = Box::pin(client.linearizable_read(&key));
            let pending = tokio::time::timeout(Duration::from_millis(100), read.as_mut()).await;
            assert!(pending.is_err());
            release_tx.send(()).unwrap();
            let response = tokio::time::timeout(TIMEOUT, completed).await.unwrap().unwrap().unwrap();
            let actual = tokio::time::timeout(TIMEOUT, read).await.unwrap().unwrap().unwrap();
            let expected = types_kv::Response::new("new", committed.index());
            assert_eq!(expected, response.data);
            assert_eq!(expected, actual);
        }
        app.raft.shutdown().await.unwrap();
        rpc.abort();
        http.abort();
    });
}

#[test]
fn readiness_reports_an_elected_leader_before_it_applies_its_noop() {
    TypeConfig::run(async {
        let (app, rpc, http, _) = node(1, 46000).await;
        let client = Client::<TypeConfig>::new(1, app.api_addr.clone());
        let key = "key".to_string();

        tracing::info!("Pause the state machine, then elect node 1, which commits its no-op without applying it");
        let release = {
            let release = pause_apply(&app).await;
            let info = Node::new(&app.raft_addr, &app.api_addr);
            app.raft.initialize(BTreeMap::from([(1, info)])).await.unwrap();
            app.raft.wait(Some(TIMEOUT)).state(ServerState::Leader, "node 1 elected").await.unwrap();
            release
        };

        tracing::info!("Readiness reports the election, while a linearizable read waits for the no-op");
        let read = {
            let readiness = Readiness {
                leader: Some(1),
                is_leader: true,
                applied_leader_noop: false,
                last_applied: None,
            };
            assert_eq!(readiness, client.readiness().await.unwrap());
            let mut read = Box::pin(client.linearizable_read(&key));
            let pending = tokio::time::timeout(Duration::from_millis(100), read.as_mut()).await;
            assert!(pending.is_err());
            read
        };

        tracing::info!("Once node 1 applies its no-op, readiness reports it and the read completes");
        {
            release.send(()).unwrap();
            let response = tokio::time::timeout(TIMEOUT, read).await.unwrap().unwrap().unwrap();
            assert_eq!(types_kv::Response::none(), response);
            let readiness = client.readiness().await.unwrap();
            assert!(readiness.applied_leader_noop);
        }
        app.raft.shutdown().await.unwrap();
        rpc.abort();
        http.abort();
    });
}

#[test]
fn follower_read_does_not_wait_for_the_leader_to_apply() {
    TypeConfig::run(async {
        let (nodes, old) = cluster(45000).await;
        let (leader, follower) = (&nodes[0].0, &nodes[1].0);
        let key = "key".to_string();

        tracing::info!("Pause the leader's state machine, then commit key=new without applying it there");
        let (release, committed, completed) = {
            let release = pause_apply(leader).await;
            let (responder, committed, completed) = ProgressResponder::new();
            leader.raft.write(types_kv::Request::set("key", "new")).responder(responder).await.unwrap();
            let committed = tokio::time::timeout(TIMEOUT, committed).await.unwrap().unwrap();
            (release, committed, completed)
        };

        tracing::info!(
            "Follower 2 reads key=new at {} while the leader still holds key=old",
            committed
        );
        {
            let leader_client = Client::<TypeConfig>::new(1, leader.api_addr.clone());
            assert_eq!(old.data, leader_client.read(&key).await.unwrap());
            let client = Client::<TypeConfig>::new(2, follower.api_addr.clone());
            let read = tokio::time::timeout(TIMEOUT, client.follower_read(&key)).await.unwrap().unwrap().unwrap();
            assert_eq!(types_kv::Response::new("new", committed.index()), read);
        }

        release.send(()).unwrap();
        tokio::time::timeout(TIMEOUT, completed).await.unwrap().unwrap().unwrap();
        shutdown(nodes).await;
    });
}

#[test]
fn follower_read_waits_for_the_follower_to_apply() {
    TypeConfig::run(async {
        let (nodes, old) = cluster(44000).await;
        let (leader, follower) = (&nodes[0].0, &nodes[1].0);
        let key = "key".to_string();

        tracing::info!("Round 1: leader 1 authorizes a read on follower 2 at {}", old.log_id);
        {
            let state = follower.ensure_follower_read_ready(TIMEOUT).await.unwrap();
            assert_eq!(&old.log_id, state.read_log_id().log_id());
        }

        tracing::info!("Pause follower 2's state machine, then write key=new through the leader");
        let (release, newer) = {
            let release = pause_apply(follower).await;
            let newer = leader.raft.client_write(types_kv::Request::set("key", "new")).await.unwrap();
            (release, newer)
        };

        tracing::info!(
            "Round 2 is authorized at {}, which follower 2 does not apply before the deadline",
            newer.log_id
        );
        {
            let err = follower.ensure_follower_read_ready(Duration::from_millis(200)).await.unwrap_err();
            let FollowerReadError::NotApplied { read_log_id, applied } = err else {
                panic!("expect NotApplied, got: {err}");
            };
            assert_eq!(newer.log_id, read_log_id);
            assert_eq!(Some(old.log_id), applied);
        }

        tracing::info!("A read with the endpoint's deadline returns key=new once follower 2 applies it");
        {
            let client = Client::<TypeConfig>::new(2, follower.api_addr.clone());
            let mut read = Box::pin(client.follower_read(&key));
            let pending = tokio::time::timeout(Duration::from_millis(100), read.as_mut()).await;
            assert!(pending.is_err());
            release.send(()).unwrap();
            let value = tokio::time::timeout(TIMEOUT, read).await.unwrap().unwrap().unwrap();
            assert_eq!(newer.data, value);
        }
        shutdown(nodes).await;
    });
}

#[test]
fn follower_read_after_a_leader_change_is_authorized_by_the_new_leader() {
    TypeConfig::run(async {
        let (nodes, old) = cluster(43000).await;
        let (node1, node2, node3) = (&nodes[0].0, &nodes[1].0, &nodes[2].0);
        let block_node3 = &nodes[2].3;
        let key = "key".to_string();

        tracing::info!("Round 1: leader 1 authorizes a read on follower 3 at {}", old.log_id);
        {
            let state = node3.ensure_follower_read_ready(TIMEOUT).await.unwrap();
            assert_eq!(&old.log_id, state.read_log_id().log_id());
        }

        tracing::info!("Cut inbound Raft traffic to node 3, elect node 2, and write key=new through it");
        let newer = {
            block_node3.store(true, Ordering::Relaxed);
            // Let leader 1's quorum-acknowledgement lease expire, so that it grants node 2 its vote.
            tokio::time::sleep(Duration::from_millis(250)).await;
            node2.raft.trigger().elect(false).await.unwrap();
            // Node 2 admits writes once node 1 has acknowledged it, as committing its no-op shows.
            let noop_index = Some(old.log_id.index() + 1);
            node2.raft.wait(Some(TIMEOUT)).applied_index(noop_index, "node 2 applies its no-op").await.unwrap();
            node1.raft.wait(Some(TIMEOUT)).current_leader(2, "node 1 follows node 2").await.unwrap();
            node2.raft.client_write(types_kv::Request::set("key", "new")).await.unwrap()
        };

        tracing::info!(
            "Round 2: node 3 still names leader 1, which redirects the read to node 2 at {}",
            newer.log_id
        );
        {
            assert_eq!(Some(1), node3.raft.current_leader().await);
            let err = node3.ensure_follower_read_ready(Duration::from_millis(500)).await.unwrap_err();
            let FollowerReadError::NotApplied { read_log_id, applied } = err else {
                panic!("expect NotApplied, got: {err}");
            };
            assert_eq!(newer.log_id, read_log_id);
            assert_eq!(Some(old.log_id), applied);
        }

        tracing::info!("Restore inbound Raft traffic to node 3, which catches up and reads key=new");
        {
            block_node3.store(false, Ordering::Relaxed);
            let client = Client::<TypeConfig>::new(3, node3.api_addr.clone());
            let value = tokio::time::timeout(TIMEOUT, client.follower_read(&key)).await.unwrap().unwrap().unwrap();
            assert_eq!(newer.data, value);
        }
        shutdown(nodes).await;
    });
}

#[test]
fn follower_read_is_not_authorized_by_a_leader_without_its_quorum() {
    TypeConfig::run(async {
        let (nodes, _old) = cluster(42000).await;
        let follower = &nodes[1].0;

        tracing::info!("Cut inbound Raft traffic to both followers, so leader 1 cannot confirm its leadership");
        for (.., blocked) in &nodes[1..] {
            blocked.store(true, Ordering::Relaxed);
        }

        tracing::info!("A deadline shorter than the leader's wait for its quorum expires before any answer");
        {
            let err = follower.ensure_follower_read_ready(Duration::from_millis(100)).await.unwrap_err();
            let FollowerReadError::NoAnswer { leader_id, .. } = err else {
                panic!("expect NoAnswer, got: {err}");
            };
            assert_eq!(1, leader_id);
        }

        tracing::info!("With a longer deadline, leader 1 stops waiting for its quorum and refuses the read");
        {
            let err = follower.ensure_follower_read_ready(TIMEOUT).await.unwrap_err();
            let FollowerReadError::Refused(refused) = err else {
                panic!("expect Refused, got: {err}");
            };
            let quorum_not_enough = matches!(refused, openraft::errors::LinearizableReadError::QuorumNotEnough(_));
            assert!(quorum_not_enough, "expect QuorumNotEnough, got: {refused}");
        }
        shutdown(nodes).await;
    });
}
