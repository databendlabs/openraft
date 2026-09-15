use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use futures::future::Either;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::NetworkError;
use openraft::errors::RPCError;
use openraft::raft::linearizable_read::LinearizerOption;
use openraft::type_config::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

const PROBE_INTERVAL: u64 = 2_000;

/// ReadIndex cannot keep the bridge leased once the old Leader becomes inactive.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn bridge_election_despite_continuous_read_index() -> Result<()> {
    for enabled in [false, true] {
        let config = Arc::new(
            Config {
                enable_elect: false,
                quorum_loss_grace: enabled.then_some(0),
                quorum_loss_probe_interval: enabled.then_some(PROBE_INTERVAL),
                ..Default::default()
            }
            .validate()?,
        );
        let mut router = RaftRouter::new(config);

        tracing::info!(enabled, "--- establish five voters with node 0 as Leader");
        let log_index = router.new_cluster(btreeset! {0, 1, 2, 3, 4}, btreeset! {}).await?;
        let n0 = router.get_raft_handle(&0)?;
        let n3 = router.get_raft_handle(&3)?;
        let n4 = router.get_raft_handle(&4)?;
        let old_vote = n0.metrics().borrow_watched().vote;

        tracing::info!(
            log_index,
            enabled,
            "--- old Leader reaches only bridge 2; voters 2, 3, 4 remain connected"
        );
        {
            router.set_network_error(1, true);
            for rpc_type in [RPCTypes::AppendEntries, RPCTypes::Vote] {
                router
                    .set_rpc_pre_hook(rpc_type, move |_router, _req, from, target| {
                        let result = if (from == 0 && target >= 3) || (target == 0 && from >= 3) {
                            Err(RPCError::Network(NetworkError::<TypeConfig>::from_string(
                                "cut between old Leader and voters 3, 4",
                            )))
                        } else {
                            Ok(())
                        };
                        Box::pin(futures::future::ready(result))
                    })
                    .await;
            }
            n3.runtime_config().elect(true);
            n4.runtime_config().elect(true);
        }

        tracing::info!(
            log_index,
            enabled,
            "--- continuously request ReadIndex while voters 3 and 4 campaign"
        );
        let reads_sent = AtomicU64::new(0);
        let stop_reads = AtomicBool::new(false);
        let observe = async {
            let result = if enabled {
                let wait3 = n3.wait(timeout());
                let wait4 = n4.wait(timeout());
                let election = futures::future::select(
                    Box::pin(wait3.state(ServerState::Leader, "voter 3 can win through bridge 2")),
                    Box::pin(wait4.state(ServerState::Leader, "voter 4 can win through bridge 2")),
                )
                .await;
                match election {
                    Either::Left((result, _)) | Either::Right((result, _)) => result.map(|_| ()),
                }
            } else {
                TypeConfig::sleep(Duration::from_millis(1_200)).await;
                Ok(())
            };
            stop_reads.store(true, Ordering::SeqCst);
            result
        };
        let read_traffic = async {
            while !stop_reads.load(Ordering::SeqCst) {
                reads_sent.fetch_add(1, Ordering::SeqCst);
                let option =
                    LinearizerOption::new(Some(Duration::ZERO), true).with_wait_timeout(Duration::from_millis(20));
                let _ = n0.get_read_linearizer(option).await;
            }
        };
        let (result, ()) = futures::future::join(observe, read_traffic).await;
        result?;
        assert!(
            reads_sent.load(Ordering::SeqCst) > 3,
            "ReadIndex traffic must overlap the election window"
        );

        tracing::info!(
            log_index,
            enabled,
            "--- inspect the policy outcome and old Leader identity"
        );
        {
            if enabled {
                let elected3 = n3.metrics().borrow_watched().state == ServerState::Leader;
                let elected4 = n4.metrics().borrow_watched().state == ServerState::Leader;
                assert!(elected3 || elected4, "the connected quorum must elect a new Leader");
                n0.wait(timeout())
                    .metrics(
                        |metrics| metrics.vote > old_vote && metrics.state != ServerState::Leader,
                        "the old Leader's probe observes the bridge's HigherVote",
                    )
                    .await?;
            } else {
                let metrics = n0.metrics().borrow_watched().clone();
                assert_eq!(old_vote, metrics.vote);
                assert_eq!(ServerState::Leader, metrics.state);
                assert_eq!(Some(0), router.get_metrics(&2)?.current_leader);
                assert_ne!(ServerState::Leader, n3.metrics().borrow_watched().state);
                assert_ne!(ServerState::Leader, n4.metrics().borrow_watched().state);
            }
        }

        for node_id in 0..5 {
            router.get_raft_handle(&node_id)?.shutdown().await?;
        }
    }
    Ok(())
}

/// Probes recover the same Leader and resume a retained stream without periodic heartbeat.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn recovery_probe_resumes_pending_replication() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_elect: false,
            enable_heartbeat: false,
            max_payload_entries: 1,
            quorum_loss_grace: Some(0),
            quorum_loss_probe_interval: Some(PROBE_INTERVAL),
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- establish three voters without periodic heartbeat or elections");
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;
    let n0 = router.get_raft_handle(&0)?;
    let _ = n0.get_read_linearizer(ReadPolicy::ReadIndex).await?;
    let old_vote = n0.metrics().borrow_watched().vote;
    let partitioned = Arc::new(AtomicBool::new(true));
    let (probe_tx, probe_rx) = TypeConfig::oneshot();
    let probe_tx = Mutex::new(Some(probe_tx));
    let probe_attempts = AtomicU64::new(0);
    let retained_requests = Arc::new(AtomicU64::new(0));
    let (release_tx, release_rx) = TypeConfig::oneshot();
    let release_rx = Mutex::new(Some(release_rx));
    let (first_sent_tx, first_sent_rx) = TypeConfig::oneshot();
    let first_sent_tx = Mutex::new(Some(first_sent_tx));

    tracing::info!(
        log_index,
        "--- retain the first stream item while blocking both followers"
    );
    {
        let partitioned = partitioned.clone();
        let retained_requests = retained_requests.clone();
        let installed_at = TypeConfig::now();
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, from, target| {
                let blocked = from == 0 && partitioned.load(Ordering::SeqCst);
                if blocked
                    && target == 1
                    && matches!(&req, RpcRequest::AppendEntries(req) if !req.entries.is_empty())
                    && retained_requests.fetch_add(1, Ordering::SeqCst) == 0
                {
                    first_sent_tx.lock().unwrap().take().unwrap().send(()).ok();
                    let release_rx = release_rx.lock().unwrap().take().unwrap();
                    return Box::pin(async move {
                        release_rx.await.unwrap();
                        Ok(())
                    });
                }
                if blocked
                    && TypeConfig::now() >= installed_at + Duration::from_millis(PROBE_INTERVAL)
                    && matches!(&req, RpcRequest::AppendEntries(req) if req.entries.is_empty())
                    && probe_attempts.fetch_add(1, Ordering::SeqCst) == 1
                    && let Some(tx) = probe_tx.lock().unwrap().take()
                {
                    tx.send(()).ok();
                }
                let result = if blocked {
                    Err(RPCError::Network(NetworkError::<TypeConfig>::from_string(
                        "partition both followers until the first recovery probe",
                    )))
                } else {
                    Ok(())
                };
                Box::pin(futures::future::ready(result))
            })
            .await;
    }
    log_index += 2;

    let pending_write = async {
        let mut writes = n0
            .client_write_many([
                ClientRequest::make_request("pending-inactive", 1),
                ClientRequest::make_request("pending-inactive", 2),
            ])
            .await?;
        while let Some(result) = writes.next().await {
            result??;
        }
        Ok::<_, anyhow::Error>(())
    };
    let recover = async {
        tracing::info!(
            log_index,
            "--- let an expired admitted RPC finish without admitting the next stream item"
        );
        {
            router
                .wait(&0, timeout())
                .metrics(
                    |metrics| metrics.last_log_index == Some(log_index),
                    "both stream items were appended before inactivity",
                )
                .await?;
            TypeConfig::timeout(Duration::from_secs(5), first_sent_rx).await??;
            TypeConfig::timeout(Duration::from_secs(5), probe_rx).await??;
            release_tx.send(()).unwrap();
            router
                .wait(&1, timeout())
                .metrics(
                    |metrics| metrics.last_log_index == Some(log_index - 1),
                    "the admitted first stream item finishes while inactive",
                )
                .await?;
            TypeConfig::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                1,
                retained_requests.load(Ordering::SeqCst),
                "the next retained stream item must wait for activity"
            );
            let metrics = n0.metrics().borrow_watched().clone();
            assert_eq!(ServerState::Leader, metrics.state, "inactive is still publicly Leader");
            assert_eq!(old_vote, metrics.vote, "inactivity does not change Vote");
            assert_eq!(Some(0), metrics.current_leader);
            partitioned.store(false, Ordering::SeqCst);
        }

        tracing::info!(
            log_index,
            "--- the next probe restores quorum and retained replication catches up"
        );
        {
            for node_id in 0..3 {
                router
                    .wait(&node_id, timeout())
                    .applied_index(Some(log_index), "pending write is replicated and applied")
                    .await?;
            }
            let metrics = n0.metrics().borrow_watched().clone();
            assert_eq!(old_vote, metrics.vote, "recovery keeps the same Leader session");
            assert_eq!(ServerState::Leader, metrics.state);
        }
        Ok::<_, anyhow::Error>(())
    };
    let (write_result, recovery_result) =
        TypeConfig::timeout(Duration::from_secs(10), futures::future::join(pending_write, recover)).await?;
    write_result?;
    recovery_result?;
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_secs(5))
}
