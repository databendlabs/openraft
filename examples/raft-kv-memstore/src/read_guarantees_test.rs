use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use app_http::Client;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::impls::ProgressResponder;
use openraft::type_config::TypeConfigExt;
use tokio::task::JoinHandle;

use super::*;

const TIMEOUT: Duration = Duration::from_secs(10);

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
        }
        let newer = next.raft.client_write(types_kv::Request::set("key", "new")).await.unwrap();
        let old_metrics = old.raft.metrics();
        let old_state = old_metrics.borrow_watched().state;
        assert_eq!(ServerState::Leader, old_state);
        let old_client = Client::<TypeConfig>::new(1, old.api_addr.clone());
        let key = "key".to_string();
        tracing::info!("Local read exposes the old value, while the linearizable endpoint rejects old leadership");
        {
            let local = old_client.read(&key).await.unwrap();
            assert_eq!(written.data, local);
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
