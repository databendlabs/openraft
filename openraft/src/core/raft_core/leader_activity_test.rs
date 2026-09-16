use std::io;
use std::ops::RangeBounds;

use maplit::btreeset;
use openraft_rt_tokio::TokioRuntime;

use super::*;
use crate::AsyncRuntime;
use crate::RaftLogReader;
use crate::RaftNetworkV2;
use crate::Vote;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::io_flush_tracking::IoProgressWatcher;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::RPCError;
use crate::errors::ReplicationClosed;
use crate::errors::StreamingError;
use crate::proposer::leader_activity::LeaderActivity;
use crate::raft::AppendEntriesResponse;
use crate::raft::SnapshotResponse;
use crate::storage::LogState;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

type TestCore = RaftCore<UTConfig, Network, UnusedLogStore, ()>;

struct UnusedLogStore;

impl RaftLogReader<UTConfig> for UnusedLogStore {
    async fn try_get_log_entries<RB>(
        &mut self,
        _range: RB,
    ) -> Result<Vec<<UTConfig as RaftTypeConfig>::Entry>, io::Error>
    where
        RB: RangeBounds<u64> + Clone + Debug + OptionalSend,
    {
        unreachable!("activity commands do not read logs")
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<UTConfig>>, io::Error> {
        unreachable!("activity commands do not read votes")
    }
}

impl RaftLogStorage<UTConfig> for UnusedLogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<UTConfig>, io::Error> {
        unreachable!("activity commands do not access storage")
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        unreachable!("activity commands do not access storage")
    }

    async fn save_vote(&mut self, _vote: &VoteOf<UTConfig>) -> Result<(), io::Error> {
        unreachable!("activity commands do not persist votes")
    }

    async fn append<I>(&mut self, _entries: I, _callback: IOFlushed<UTConfig>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = <UTConfig as RaftTypeConfig>::Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        unreachable!("activity commands do not append logs")
    }

    async fn truncate_after(&mut self, _after: Option<LogIdOf<UTConfig>>) -> Result<(), io::Error> {
        unreachable!("activity commands do not truncate logs")
    }

    async fn purge(&mut self, _upto: LogIdOf<UTConfig>) -> Result<(), io::Error> {
        unreachable!("activity commands do not purge logs")
    }
}

#[derive(Clone)]
struct Network {
    target: u64,
    sent: MpscSenderOf<UTConfig, u64>,
}

impl RaftNetworkFactory<UTConfig> for Network {
    type Network = Self;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        Self { target, ..self.clone() }
    }
}

impl RaftNetworkV2<UTConfig> for Network {
    type SnapshotData = ();

    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<UTConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<UTConfig>, RPCError<UTConfig>> {
        self.sent.send(self.target).await.ok();
        Ok(AppendEntriesResponse::Success)
    }

    async fn vote(
        &mut self,
        _rpc: VoteRequest<UTConfig>,
        _option: RPCOption,
    ) -> Result<VoteResponse<UTConfig>, RPCError<UTConfig>> {
        unreachable!("activity commands do not send votes")
    }

    async fn full_snapshot(
        &mut self,
        _vote: VoteOf<UTConfig>,
        _snapshot: SnapshotOf<UTConfig, ()>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<UTConfig>, StreamingError<UTConfig>> {
        unreachable!("probe commands do not send snapshots")
    }
}

fn core() -> (TestCore, MpscReceiverOf<UTConfig, u64>) {
    let config = Arc::new(Config {
        quorum_loss_probe_interval: Some(700),
        ..Default::default()
    });
    let mut engine = Engine::testing_default(0);
    engine.config = EngineConfig::new(0, &config);
    engine.state.enable_validation(false);
    let membership = Membership::new_with_defaults(vec![btreeset! {0, 1, 2}], [3]);
    let stored = Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership));
    engine.state.membership_state = MembershipStateOf::<UTConfig>::new(stored.clone(), stored);
    engine.state.vote = Leased::new(UTConfig::<()>::now(), Duration::ZERO, Vote::new_committed(1, 0));
    engine.state.log_ids.append(log_id(1, 0, 1));
    engine.state.server_state = ServerState::Leader;
    engine.testing_new_leader();
    let (sent, received) = UTConfig::<()>::mpsc(8);
    let (tx_api, rx_api) = UTConfig::<()>::mpsc(8);
    let (tx_install_snapshot, rx_install_snapshot) = UTConfig::<()>::mpsc(8);
    let (tx_notification, rx_notification) = UTConfig::<()>::mpsc(8);
    let default_io = IOId::new_vote_io(Vote::new(0, 0).to_non_committed());
    let core = RaftCore {
        id: 0,
        config: config.clone(),
        runtime_config: Arc::new(RuntimeConfig::new(&config)),
        core_state: Default::default(),
        network_factory: Arc::new(UTConfig::<()>::mutex(Network { target: 0, sent })),
        log_store: UnusedLogStore,
        sm_handle: sm::worker::Worker::spawn(0, (), UnusedLogStore, tx_notification.clone(), 8, Span::none()),
        engine,
        client_responders: ClientResponderQueue::with_capacity(0),
        pending_reads: Default::default(),
        pending_read_deadline_notifier: PendingReadDeadlineNotifier::spawn(tx_notification.clone()),
        replications: Default::default(),
        heartbeat_handle: HeartbeatWorkersHandle::new(0, config.clone()),
        activity_tx: Some(UTConfig::<()>::watch_channel(false).0),
        tx_api,
        rx_api: BatchRaftMsgReceiver::new(rx_api, 8, Duration::ZERO),
        tx_install_snapshot,
        rx_install_snapshot,
        tx_notification,
        rx_notification,
        io_broadcast: IoBroadcast {
            completed: UTConfig::<()>::watch_channel(Ok(default_io.clone())).0,
            accepted: UTConfig::<()>::watch_channel(default_io.clone()).0,
            submitted: UTConfig::<()>::watch_channel(default_io).0,
            committed: UTConfig::<()>::watch_channel(None).0,
        },
        metrics: MetricsChannels {
            all: UTConfig::<()>::watch_channel(RaftMetrics::new_initial(0)).0,
            data: UTConfig::<()>::watch_channel(RaftDataMetrics::default()).0,
            server: UTConfig::<()>::watch_channel(RaftServerMetrics::new_initial(0)).0,
            progress: IoProgressWatcher::new().0,
        },
        runtime_stats: RuntimeStats::new(&config),
        shared_replicate_batch: SharedReplicateBatch::new(),
        metrics_recorder: None,
        span: Span::none(),
    };
    (core, received)
}

fn probe_deadline(core: &TestCore) -> InstantOf<UTConfig> {
    match core.engine.leader.as_ref().unwrap().activity.unwrap() {
        LeaderActivity::Inactive { next_probe_at } => next_probe_at,
        state => panic!("expected Inactive, got {state:?}"),
    }
}

#[test]
fn tick_closes_activity_before_ordinary_heartbeat() {
    TokioRuntime::new(1).block_on(async {
        let (mut core, _received) = core();
        core.engine.output.take_commands();

        let due = UTConfig::<()>::now() - Duration::from_millis(1);
        let leader = core.engine.leader.as_mut().unwrap();
        leader.activity = Some(LeaderActivity::Active { next_check_at: due });
        leader.next_heartbeat = due;

        tracing::info!("Close ordinary traffic before processing a heartbeat due on the same tick");
        {
            core.handle_tick(1);
            assert!(core.engine.leader.as_ref().unwrap().is_inactive());
            assert!(matches!(core.engine.output.take_commands().as_slice(), [
                Command::SetLeaderActivity { active: false, .. }
            ]));
        }
    });
}

#[test]
fn closure_anchors_first_probe_to_core_execution() {
    TokioRuntime::new(1).block_on(async {
        let (mut core, _received) = core();
        let interval = Duration::from_millis(700);
        let old = UTConfig::<()>::now() - Duration::from_secs(2);
        core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive { next_probe_at: old });
        let _permit = core.activity_tx.as_ref().unwrap().subscribe();
        core.activity_tx.as_ref().unwrap().send(true).unwrap();

        tracing::info!("Delayed admission closure must grant a full R from actual Core execution");
        {
            let before = UTConfig::<()>::now();
            core.set_leader_activity(Vote::new(1, 0).to_committed(), false);
            let after = UTConfig::<()>::now();
            assert!(!*core.activity_tx.as_ref().unwrap().borrow_watched());
            assert!(before + interval <= probe_deadline(&core));
            assert!(probe_deadline(&core) <= after + interval);
        }
    });
}

#[test]
fn probe_submission_is_atomic_and_rearms_from_execution() {
    TokioRuntime::new(1).block_on(async {
        let (mut core, mut received) = core();
        let targets = core
            .engine
            .leader
            .as_ref()
            .unwrap()
            .progress
            .iter()
            .filter(|entry| entry.id != 0)
            .map(|entry| TargetProgress {
                target: entry.id,
                target_node: (),
                progress: entry.clone(),
            })
            .collect::<Vec<_>>();
        core.heartbeat_handle
            .spawn_workers(
                Vote::new(1, 0).to_committed(),
                &mut *core.network_factory.lock().await,
                &core.tx_notification,
                targets.iter(),
                false,
            )
            .await;
        let old = UTConfig::<()>::now() - Duration::from_secs(2);
        core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive { next_probe_at: old });

        tracing::info!("A stale voter stream must prevent the entire round, including ready voters");
        {
            let stream = core.engine.leader.as_ref().unwrap().progress.try_get(&2).unwrap().data.stream_id;
            core.engine
                .leader
                .as_mut()
                .unwrap()
                .progress
                .iter_mut_without_reorder()
                .find(|entry| entry.id == 2)
                .unwrap()
                .data
                .stream_id = StreamId::new(*stream + 100);
            core.submit_quorum_probe(Vote::new(1, 0).to_committed());
            assert_eq!(old, probe_deadline(&core));
            assert!(UTConfig::<()>::timeout(Duration::from_millis(10), received.recv()).await.is_err());
            core.engine
                .leader
                .as_mut()
                .unwrap()
                .progress
                .iter_mut_without_reorder()
                .find(|entry| entry.id == 2)
                .unwrap()
                .data
                .stream_id = stream;
        }

        tracing::info!("A submitted round reaches voters only and grants a full R before another round");
        {
            let before = UTConfig::<()>::now();
            core.submit_quorum_probe(Vote::new(1, 0).to_committed());
            let after = UTConfig::<()>::now();
            let deadline = probe_deadline(&core);
            assert!(before + Duration::from_millis(700) <= deadline);
            assert!(deadline <= after + Duration::from_millis(700));
            let first = UTConfig::<()>::timeout(Duration::from_secs(1), received.recv()).await.unwrap().unwrap();
            let second = UTConfig::<()>::timeout(Duration::from_secs(1), received.recv()).await.unwrap().unwrap();
            assert_eq!(btreeset! {1, 2}, btreeset! {first, second});
            core.submit_quorum_probe(Vote::new(1, 0).to_committed());
            assert_eq!(deadline, probe_deadline(&core));
            assert!(UTConfig::<()>::timeout(Duration::from_millis(10), received.recv()).await.is_err());
        }

        tracing::info!("A missing voter worker also leaves the due round completely unsent");
        {
            core.heartbeat_handle.workers.remove(&2);
            core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive { next_probe_at: old });
            core.submit_quorum_probe(Vote::new(1, 0).to_committed());
            assert_eq!(old, probe_deadline(&core));
            assert!(UTConfig::<()>::timeout(Duration::from_millis(10), received.recv()).await.is_err());
        }
    });
}

#[test]
fn obsolete_activity_commands_do_not_resume_or_probe() {
    TokioRuntime::new(1).block_on(async {
        let (mut core, mut received) = core();
        let targets = core
            .engine
            .leader
            .as_ref()
            .unwrap()
            .progress
            .iter()
            .filter(|entry| entry.id != 0)
            .map(|entry| TargetProgress {
                target: entry.id,
                target_node: (),
                progress: entry.clone(),
            })
            .collect::<Vec<_>>();
        core.heartbeat_handle
            .spawn_workers(
                Vote::new(1, 0).to_committed(),
                &mut *core.network_factory.lock().await,
                &core.tx_notification,
                targets.iter(),
                false,
            )
            .await;
        let old = UTConfig::<()>::now() - Duration::from_secs(2);
        core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive { next_probe_at: old });

        tracing::info!("Suppress an ordinary heartbeat queued before inactivity");
        {
            core.broadcast_heartbeat(ReplicationSessionId::new(Vote::new(1, 0).to_committed(), None), true);
            assert!(UTConfig::<()>::timeout(Duration::from_millis(10), received.recv()).await.is_err());
        }

        tracing::info!("Wrong authority and a recovered-again-inactive session invalidate queued commands");
        {
            core.set_leader_activity(Vote::new(2, 0).to_committed(), false);
            core.set_leader_activity(Vote::new(2, 0).to_committed(), true);
            core.set_leader_activity(Vote::new(1, 0).to_committed(), true);
            core.submit_quorum_probe(Vote::new(2, 0).to_committed());
            assert_eq!(old, probe_deadline(&core));
            assert!(!*core.activity_tx.as_ref().unwrap().borrow_watched());
            assert!(UTConfig::<()>::timeout(Duration::from_millis(10), received.recv()).await.is_err());
        }

        tracing::info!("A recovered session rejects queued closure and probe but permits its current opening");
        {
            core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Active {
                next_check_at: UTConfig::<()>::now() + Duration::from_millis(300),
            });
            core.set_leader_activity(Vote::new(2, 0).to_committed(), true);
            assert!(!*core.activity_tx.as_ref().unwrap().borrow_watched());
            core.set_leader_activity(Vote::new(1, 0).to_committed(), false);
            core.submit_quorum_probe(Vote::new(1, 0).to_committed());
            assert!(matches!(
                core.engine.leader.as_ref().unwrap().activity,
                Some(LeaderActivity::Active { .. })
            ));
            assert!(!*core.activity_tx.as_ref().unwrap().borrow_watched());
            core.set_leader_activity(Vote::new(1, 0).to_committed(), true);
            assert!(*core.activity_tx.as_ref().unwrap().borrow_watched());
            core.engine.leader = None;
            core.set_leader_activity(Vote::new(1, 0).to_committed(), false);
            core.submit_quorum_probe(Vote::new(1, 0).to_committed());
            assert!(*core.activity_tx.as_ref().unwrap().borrow_watched());
            assert!(received.try_recv().is_err());
        }
    });
}

#[test]
fn activity_channel_follows_leader_sessions() {
    TokioRuntime::new(1).block_on(async {
        let (mut core, _received) = core();
        let vote = Vote::new(1, 0).to_committed();
        let mut permit = core.activity_tx.as_ref().unwrap().subscribe();
        core.activity_tx.as_ref().unwrap().send(true).unwrap();
        assert!(permit.changed().now_or_never().unwrap().is_ok());
        assert!(*permit.borrow_and_update());

        tracing::info!("A membership rebuild reuses and updates the current Leader session permit");
        {
            core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive {
                next_probe_at: UTConfig::<()>::now(),
            });
            core.run_rebuild_replication_streams(vote.clone(), vec![], false).await;
            assert!(permit.changed().now_or_never().unwrap().is_ok());
            assert!(!*permit.borrow_and_update());

            core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Active {
                next_check_at: UTConfig::<()>::now() + Duration::from_millis(300),
            });
            core.run_rebuild_replication_streams(vote.clone(), vec![], false).await;
            assert!(permit.changed().now_or_never().unwrap().is_ok());
            assert!(*permit.borrow_and_update());
        }

        tracing::info!("A hard rebuild closes the old permit and initializes the new value from activity");
        {
            core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Inactive {
                next_probe_at: UTConfig::<()>::now(),
            });
            core.run_rebuild_replication_streams(vote.clone(), vec![], true).await;
            assert!(permit.changed().now_or_never().unwrap().is_err());
            assert!(!*core.activity_tx.as_ref().unwrap().borrow_watched());

            let mut inactive_permit = core.activity_tx.as_ref().unwrap().subscribe();
            core.engine.leader.as_mut().unwrap().activity = Some(LeaderActivity::Active {
                next_check_at: UTConfig::<()>::now() + Duration::from_millis(300),
            });
            core.run_rebuild_replication_streams(vote, vec![], true).await;
            assert!(inactive_permit.changed().now_or_never().unwrap().is_err());
            assert!(*core.activity_tx.as_ref().unwrap().borrow_watched());
        }

        tracing::info!("Closing the Leader session closes its permit channel");
        {
            let mut current = core.activity_tx.as_ref().unwrap().subscribe();
            core.run_close_replication_streams();
            assert!(current.changed().now_or_never().unwrap().is_err());
        }
    });
}
