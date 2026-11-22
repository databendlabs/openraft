use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreemap;
use maplit::btreeset;

use super::StepDownWatcher;
use super::is_removed_leader;
use super::transfer_target;
use crate::Config;
use crate::Membership;
use crate::StepDownPolicy;
use crate::Vote;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::watch::WatchSender;
use crate::core::ServerState;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::leader_removal::LeaderRemoval;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;

/// Voters 2,3; node 1 is removed.
fn m23() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], [])
}

fn stored(log_index: u64, mem: Membership<u64, ()>) -> Arc<StoredMembershipOf<UTConfig>> {
    Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, log_index)), mem))
}

/// Build server metrics of a Leader node 1, with the same effective and committed
/// membership config.
fn server_metrics(mem: Membership<u64, ()>) -> RaftServerMetrics<UTConfig> {
    let mut m = RaftServerMetrics::new_initial(1);
    m.state = ServerState::Leader;
    m.membership_config = stored(3, mem.clone());
    m.committed_membership_config = stored(3, mem);
    m
}

#[test]
fn test_is_removed_leader_not_leader() {
    let mut m = server_metrics(m23());
    m.state = ServerState::Follower;

    assert!(!is_removed_leader(&m));
}

#[test]
fn test_is_removed_leader_voter_leader() {
    let m = server_metrics(Membership::new_with_defaults(vec![btreeset! {1,2,3}], []));

    assert!(!is_removed_leader(&m));
}

#[test]
fn test_is_removed_leader_learner_leader() {
    let m = server_metrics(Membership::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1}));

    assert!(!is_removed_leader(&m));
}

#[test]
fn test_is_removed_leader_uncommitted_membership() {
    let mut m = server_metrics(m23());
    m.committed_membership_config = stored(1, Membership::new_with_defaults(vec![btreeset! {1,2}], []));

    assert!(!is_removed_leader(&m));
}

#[test]
fn test_is_removed_leader_committed_membership() {
    let m = server_metrics(m23());

    assert!(is_removed_leader(&m));
}

#[test]
fn test_transfer_target_not_leader() {
    let m = RaftMetrics::<UTConfig>::new_initial(1);

    assert_eq!(None, transfer_target(&m));
}

#[test]
fn test_transfer_target_greatest_matching() {
    let mut m = RaftMetrics::<UTConfig>::new_initial(1);
    m.membership_config = stored(3, m23());
    m.replication = Some(btreemap! {
        2u64 => Some(log_id(2, 1, 2)),
        3u64 => Some(log_id(2, 1, 3)),
    });

    assert_eq!(Some(3), transfer_target(&m));

    m.replication = Some(btreemap! {
        2u64 => Some(log_id(2, 1, 3)),
        3u64 => None,
    });

    assert_eq!(Some(2), transfer_target(&m));
}

/// Server metrics of Leader node 1 with the vote of `term`, whose effective and committed
/// membership config is `mem` at `log_index`.
fn leader_at(term: u64, log_index: u64, mem: Membership<u64, ()>) -> RaftServerMetrics<UTConfig> {
    let mut m = server_metrics(mem.clone());
    m.vote = Vote::new_committed(term, 1);
    m.membership_config = stored(log_index, mem.clone());
    m.committed_membership_config = stored(log_index, mem);
    m
}

/// Run `body` on a current-thread runtime with a paused clock, which moves only when every
/// task waits for a timer. It fails if `body` does not finish within a minute of that clock.
///
/// `body` runs in a `LocalSet`, which `C::spawn()` needs with the `single-threaded` feature.
fn run_paused<F>(body: F)
where F: Future<Output = ()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("build a current-thread runtime with a paused clock");
    let local = tokio::task::LocalSet::new();
    let finished = local.block_on(&runtime, async {
        tokio::time::timeout(Duration::from_secs(60), body).await
    });
    finished.expect("the test body hung");
}

/// Run the watcher with `After(1000)` and a 50 ms `heartbeat_interval`. Node 1 is a removed
/// Leader at 0 ms, in term 2 at membership log index 3, and the server metrics change to each
/// of `updates` at its time in ms. No replication metrics name a transfer target, so the only
/// command is the fallback: it must come at `fallback_at` ms, fenced by the vote of `term` and
/// the membership config at `log_index`, and no other command may follow.
fn assert_fallback_at(updates: Vec<(u64, RaftServerMetrics<UTConfig>)>, fallback_at: u64, term: u64, log_index: u64) {
    run_paused(async move {
        let config = Config {
            removed_leader_step_down: StepDownPolicy::After(1_000),
            heartbeat_interval: 50,
            ..Default::default()
        };
        let start = tokio::time::Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);

        let (tx_server, rx_server) = UTConfig::<()>::watch_channel(leader_at(2, 3, m23()));
        let (_tx_metrics, rx_metrics) = UTConfig::<()>::watch_channel(RaftMetrics::new_initial(1));
        let (tx_api, mut rx_api) = UTConfig::<()>::mpsc::<RaftMsg<UTConfig>>(16);
        StepDownWatcher::<UTConfig>::spawn(rx_server, rx_metrics, tx_api.downgrade(), &config);

        for (ms, metrics) in updates {
            tokio::time::sleep_until(at(ms)).await;
            tx_server.send(metrics).unwrap();
        }

        tokio::time::sleep_until(at(fallback_at - 1)).await;
        assert!(rx_api.try_recv().is_err(), "no command before {fallback_at} ms");

        tokio::time::sleep_until(at(fallback_at + 1)).await;
        let msg = rx_api.try_recv().expect("the fallback command");
        let RaftMsg::ExternalCommand {
            cmd:
                ExternalCommand::RefreshServerState {
                    vote,
                    membership_log_id,
                },
        } = msg
        else {
            panic!("expected the fallback command, got: {msg}");
        };
        assert_eq!(Some(Vote::new_committed(term, 1)), vote);
        assert_eq!(Some(log_id(2, 1, log_index)), membership_log_id);

        tokio::time::sleep_until(at(fallback_at + 5_000)).await;
        assert!(rx_api.try_recv().is_err(), "no other command");
    });
}

#[test]
fn test_step_down_waits_the_delay_of_the_removal() {
    assert_fallback_at(vec![], 1_050, 2, 3);
}

/// A metrics update that keeps the vote and the removing membership config keeps the deadline.
#[test]
fn test_unrelated_update_keeps_the_delay() {
    let mut update = leader_at(2, 3, m23());
    update.current_leader = Some(1);
    assert_fallback_at(vec![(600, update)], 1_050, 2, 3);
}

/// A readmission ends the first removal's delay, and the second removal waits its own.
#[test]
fn test_second_removal_after_readmission_waits_its_own_delay() {
    let readmitted = leader_at(
        2,
        4,
        Membership::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1}),
    );
    let removed_again = leader_at(2, 5, m23());
    assert_fallback_at(vec![(300, readmitted), (600, removed_again)], 1_650, 2, 5);
}

/// A second removal waits its own delay even if the watcher never sees the readmission between
/// them, because the removing membership config changed.
#[test]
fn test_coalesced_second_removal_waits_its_own_delay() {
    assert_fallback_at(vec![(600, leader_at(2, 5, m23()))], 1_650, 2, 5);
}

/// The same removal under a new vote waits a delay of its own.
#[test]
fn test_removal_under_a_new_vote_waits_its_own_delay() {
    assert_fallback_at(vec![(600, leader_at(3, 3, m23()))], 1_650, 3, 3);
}

/// With replication metrics that name a target, the watcher transfers leadership at the end of the
/// delay, and the transfer carries the removal: the vote and the membership config log id that
/// fence the fallback too.
#[test]
fn test_transfer_carries_the_removal() {
    run_paused(async move {
        let config = Config {
            removed_leader_step_down: StepDownPolicy::After(1_000),
            heartbeat_interval: 50,
            ..Default::default()
        };
        let start = tokio::time::Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);

        let mut metrics = RaftMetrics::new_initial(1);
        metrics.membership_config = stored(3, m23());
        metrics.replication = Some(btreemap! {
            2u64 => Some(log_id(2, 1, 3)),
            3u64 => None,
        });

        let (_tx_server, rx_server) = UTConfig::<()>::watch_channel(leader_at(2, 3, m23()));
        let (_tx_metrics, rx_metrics) = UTConfig::<()>::watch_channel(metrics);
        let (tx_api, mut rx_api) = UTConfig::<()>::mpsc::<RaftMsg<UTConfig>>(16);
        StepDownWatcher::<UTConfig>::spawn(rx_server, rx_metrics, tx_api.downgrade(), &config);

        let expected = LeaderRemoval {
            vote: Vote::new_committed(2, 1),
            membership_log_id: Some(log_id(2, 1, 3)),
        };

        tracing::info!("--- the transfer comes at the end of the delay, fenced by the removal");
        {
            tokio::time::sleep_until(at(999)).await;
            assert!(rx_api.try_recv().is_err(), "no command before 1,000 ms");

            tokio::time::sleep_until(at(1_001)).await;
            let msg = rx_api.try_recv().expect("the transfer command");
            let RaftMsg::ExternalCommand {
                cmd: ExternalCommand::TriggerTransferLeader { to, removal },
            } = msg
            else {
                panic!("expected the transfer command, got: {msg}");
            };
            assert_eq!(2, to);
            assert_eq!(Some(expected.clone()), removal);
        }

        tracing::info!("--- the fallback follows after the heartbeat interval, with the same fence");
        {
            tokio::time::sleep_until(at(1_051)).await;
            let msg = rx_api.try_recv().expect("the fallback command");
            let RaftMsg::ExternalCommand {
                cmd:
                    ExternalCommand::RefreshServerState {
                        vote,
                        membership_log_id,
                    },
            } = msg
            else {
                panic!("expected the fallback command, got: {msg}");
            };
            assert_eq!(Some(expected.vote), vote);
            assert_eq!(expected.membership_log_id, membership_log_id);
        }
    });
}

/// Run the watcher like `assert_fallback_at()`, with an API channel of one slot, which `block_api`
/// fills, so that the fallback command at 1,050 ms waits to be sent. At `close_at` ms, right after
/// an update that the watcher has not seen, drop the server metrics sender, as a terminated
/// `RaftCore` does. The watcher must quit within 1 ms: it drops its receiver of the full metrics,
/// before any timer of its own fires, and it sends no command.
fn assert_quits_when_closed(close_at: u64, block_api: bool) {
    run_paused(async move {
        let config = Config {
            removed_leader_step_down: StepDownPolicy::After(1_000),
            heartbeat_interval: 50,
            ..Default::default()
        };
        let start = tokio::time::Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);

        let (tx_server, rx_server) = UTConfig::<()>::watch_channel(leader_at(2, 3, m23()));
        let (tx_metrics, rx_metrics) = UTConfig::<()>::watch_channel(RaftMetrics::new_initial(1));
        let (tx_api, mut rx_api) = UTConfig::<()>::mpsc::<RaftMsg<UTConfig>>(1);
        if block_api {
            let filler = RaftMsg::ExternalCommand {
                cmd: ExternalCommand::Heartbeat,
            };
            tx_api.send(filler).await.unwrap();
        }
        StepDownWatcher::<UTConfig>::spawn(rx_server, rx_metrics, tx_api.downgrade(), &config);

        let mut update = leader_at(2, 3, m23());
        update.current_leader = Some(1);

        tracing::info!(close_at, "--- metrics updates keep the watcher running");
        {
            tokio::time::sleep_until(at(close_at - 10)).await;
            tx_server.send(update.clone()).unwrap();
            tokio::time::sleep_until(at(close_at - 5)).await;
            assert!(tx_metrics.send(RaftMetrics::new_initial(1)).is_ok(), "the watcher runs");
        }

        tracing::info!(
            close_at,
            "--- RaftCore drops the server metrics sender after a last update"
        );
        {
            tokio::time::sleep_until(at(close_at)).await;
            tx_server.send(update).unwrap();
            drop(tx_server);

            tokio::time::sleep_until(at(close_at + 1)).await;
            assert!(
                tx_metrics.send(RaftMetrics::new_initial(1)).is_err(),
                "the watcher quit"
            );

            drop(tx_api);
            if block_api {
                let msg = rx_api.recv().await;
                let is_filler = matches!(
                    msg,
                    Some(RaftMsg::ExternalCommand {
                        cmd: ExternalCommand::Heartbeat
                    })
                );
                assert!(is_filler, "only the filler is in the channel");
            }
            assert!(rx_api.recv().await.is_none(), "no command is sent");
        }
    });
}

/// The core terminates during the step-down delay.
#[test]
fn test_quit_during_the_step_down_delay() {
    assert_quits_when_closed(500, false);
}

/// The core terminates after the transfer, while the watcher waits to send the fallback.
#[test]
fn test_quit_during_the_fallback_delay() {
    assert_quits_when_closed(1_020, false);
}

/// The core terminates while the fallback waits for room in the API channel.
#[test]
fn test_quit_during_a_blocked_send() {
    assert_quits_when_closed(1_100, true);
}
