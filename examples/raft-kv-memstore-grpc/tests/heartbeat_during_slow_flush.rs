//! A follower whose log flush is slow keeps answering the leader's heartbeats in time.
//!
//! While its flushes are held, the follower stops reading the leader's replication stream, and the
//! data it has not read fills the HTTP/2 connection that carries the stream. The leader's network
//! sends heartbeats on a connection of their own, so they keep arriving, and the follower answers
//! each one at once. The test reports how long heartbeats took, apart from how long the stalled
//! appends took and how long each flush was held.

#![allow(clippy::uninlined_format_args)]

use std::fmt::Debug;
use std::io;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::TryStreamExt;
use maplit::btreemap;
use openraft::Config;
use openraft::OptionalSend;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::raft::WriteOption;
use openraft::storage::RaftLogReader;
use openraft::storage::RaftLogStorage;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::InstantOf;
use raft_kv_memstore_grpc::LogStore;
use raft_kv_memstore_grpc::NodeId;
use raft_kv_memstore_grpc::StateMachineStore;
use raft_kv_memstore_grpc::grpc::raft_service::RaftServiceImpl;
use raft_kv_memstore_grpc::network::Network;
use raft_kv_memstore_grpc::protobuf as pb;
use raft_kv_memstore_grpc::protobuf::raft_service_server::RaftServiceServer;
use raft_kv_memstore_grpc::typ::*;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;

/// The leader's heartbeat interval, in milliseconds.
const HEARTBEAT_INTERVAL: u64 = 100;

/// How long node 2 holds each log flush while the test slows it down.
const FLUSH_DELAY: Duration = Duration::from_secs(5);

/// How long the test watches heartbeats while node 2 holds its flushes.
const WATCH: Duration = Duration::from_secs(2);

/// The entries the leader writes while node 2 is slow, and the size of each value: 4 MiB in all,
/// far more than node 2 reads before it stops, plus what its connection's window holds.
const WRITES: usize = 256;
const VALUE_SIZE: usize = 16 * 1024;

/// The HTTP/2 connection window of each node's server, smaller than a stream's window, 1 MiB by
/// hyper's default.
///
/// The replication stream that node 2 stops reading then uses up the window of its whole
/// connection. With hyper's default windows, which are equal, how much room the stalled stream
/// leaves depends on timing, and heartbeats on the same connection get through in some runs only.
const CONNECTION_WINDOW: u32 = 256 * 1024;

/// A log store that makes each append readable at once, but reports it flushed only after a
/// delay, as a store does whose disk is slow.
#[derive(Clone, Default)]
struct SlowFlushLog {
    inner: LogStore,
    /// The flush delay in milliseconds; with 0, an append is flushed at once.
    flush_delay_ms: Arc<AtomicU64>,
}

impl SlowFlushLog {
    fn set_flush_delay(&self, delay: Duration) {
        self.flush_delay_ms.store(delay.as_millis() as u64, Ordering::Relaxed);
    }
}

impl RaftLogReader<TypeConfig> for SlowFlushLog {
    async fn try_get_log_entries<RB>(&mut self, range: RB) -> Result<Vec<Entry>, io::Error>
    where RB: RangeBounds<u64> + Clone + Debug + OptionalSend {
        self.inner.try_get_log_entries(range).await
    }

    async fn read_vote(&mut self) -> Result<Option<Vote>, io::Error> {
        self.inner.read_vote().await
    }
}

impl RaftLogStorage<TypeConfig> for SlowFlushLog {
    type LogReader = LogStore;

    async fn get_log_state(&mut self) -> Result<LogState, io::Error> {
        self.inner.get_log_state().await
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.inner.get_log_reader().await
    }

    async fn save_vote(&mut self, vote: &Vote) -> Result<(), io::Error> {
        self.inner.save_vote(vote).await
    }

    async fn save_committed(&mut self, committed: Option<LogId>) -> Result<(), io::Error> {
        self.inner.save_committed(committed).await
    }

    async fn read_committed(&mut self) -> Result<Option<LogId>, io::Error> {
        self.inner.read_committed().await
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        self.inner.append(entries, IOFlushed::noop()).await?;

        let delay = Duration::from_millis(self.flush_delay_ms.load(Ordering::Relaxed));
        if delay.is_zero() {
            callback.io_completed(Ok(()));
        } else {
            #[allow(clippy::let_underscore_future)]
            let _ = TypeConfig::spawn(async move {
                TypeConfig::sleep(delay).await;
                callback.io_completed(Ok(()));
            });
        }
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogId>) -> Result<(), io::Error> {
        self.inner.truncate_after(last_log_id).await
    }

    async fn purge(&mut self, log_id: LogId) -> Result<(), io::Error> {
        self.inner.purge(log_id).await
    }
}

/// Start node `id` on `log`, serving its Raft service on a port of its own.
async fn start_node<LS>(id: NodeId, config: Arc<Config>, log: LS) -> anyhow::Result<(Raft, pb::Node)>
where LS: RaftLogStorage<TypeConfig> {
    let incoming = TcpIncoming::bind("127.0.0.1:0".parse()?)?;
    let node = pb::Node {
        node_id: id,
        rpc_addr: incoming.local_addr()?.to_string(),
    };

    let state_machine = Arc::new(StateMachineStore::default());
    let raft = Raft::new(id, config, Network::default(), log, state_machine).await?;

    let service = RaftServiceServer::new(RaftServiceImpl::new(raft.clone()));
    let mut server = Server::builder().initial_connection_window_size(CONNECTION_WINDOW);
    #[allow(clippy::let_underscore_future)]
    let _ = TypeConfig::spawn(server.add_service(service).serve_with_incoming(incoming));

    Ok((raft, node))
}

/// The time node 1 sent the latest request that `target` answered, heartbeat or append.
fn acked_send_time(metrics: &RaftMetrics, target: NodeId) -> Option<InstantOf<TypeConfig>> {
    let heartbeat = metrics.heartbeat.as_ref()?;
    heartbeat.get(&target)?.as_ref().map(|sent| **sent)
}

/// The index of the last log `target` has answered that it flushed.
fn matched_index(metrics: &RaftMetrics, target: NodeId) -> Option<u64> {
    let replication = metrics.replication.as_ref()?;
    replication.get(&target)?.as_ref().map(|log_id| log_id.index)
}

#[test]
fn heartbeats_reach_a_follower_whose_log_flush_is_slow() -> anyhow::Result<()> {
    TypeConfig::run(heartbeats_during_slow_flush())
}

async fn heartbeats_during_slow_flush() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: HEARTBEAT_INTERVAL,
            election_timeout_min: 1_000,
            election_timeout_max: 2_000,
            // One entry per append request: the requests node 2 has read but not answered hold
            // little data, and the rest waits unread on its connection.
            max_payload_entries: 1,
            ..Default::default()
        }
        .validate()?,
    );

    let (raft1, node1) = start_node(1, config.clone(), LogStore::default()).await?;
    let log2 = SlowFlushLog::default();
    let (_raft2, node2) = start_node(2, config, log2.clone()).await?;

    println!("--- initialize node 1, and add node 2 as a learner while node 2 flushes at once");
    {
        raft1.initialize(btreemap! {1 => node1}).await?;
        raft1.wait(Some(Duration::from_secs(5))).state(ServerState::Leader, "node 1 leads").await?;
        raft1
            .wait(Some(Duration::from_secs(5)))
            .applied_index(Some(1), "node 1 commits its blank log")
            .await?;
        raft1.add_learner(2, node2, true).await?;
    }

    println!(
        "--- hold node 2's flushes for {:?}, and write {} entries of {} bytes in one batch",
        FLUSH_DELAY, WRITES, VALUE_SIZE
    );
    let (slow_since, last_index) = {
        log2.set_flush_delay(FLUSH_DELAY);
        let slow_since = TypeConfig::now();

        // One batch reaches node 1's log at once, so that one replication session sends all of it.
        // Single writes would let the session catch up with the log and end early.
        let value = "x".repeat(VALUE_SIZE);
        let sets = (0..WRITES).map(|i| pb::SetRequest {
            key: format!("key-{}", i),
            value: value.clone(),
        });
        let mut results = raft1.client_write_many(sets, WriteOption::default()).await?;

        let mut last_index = 0;
        while let Some(result) = results.try_next().await? {
            last_index = result?.log_id.index;
        }
        (slow_since, last_index)
    };

    println!("--- watch node 2 answer node 1's heartbeats for {:?}", WATCH);
    let (round_trips, longest_wait) = {
        let watch_start = TypeConfig::now();
        let watch_end = watch_start + WATCH;
        // Node 2 answers a held append only once its flush ends, so until then every answer is a
        // heartbeat's.
        anyhow::ensure!(
            watch_end < slow_since + FLUSH_DELAY,
            "the writes took {:?}, too long to watch only heartbeats",
            watch_start - slow_since
        );

        let mut metrics = raft1.metrics();
        let mut last_sent = acked_send_time(&metrics.borrow_watched(), 2);
        let mut last_answer = watch_start;
        let mut longest_wait = Duration::ZERO;
        let mut round_trips = vec![];

        loop {
            match TypeConfig::timeout_at(watch_end, metrics.changed()).await {
                Ok(changed) => changed?,
                Err(_elapsed) => break,
            }

            let sent = acked_send_time(&metrics.borrow_watched(), 2);
            if sent == last_sent {
                continue;
            }
            last_sent = sent;

            let now = TypeConfig::now();
            if let Some(sent) = sent {
                round_trips.push(now - sent);
            }
            longest_wait = longest_wait.max(now - last_answer);
            last_answer = now;
        }
        longest_wait = longest_wait.max(watch_end - last_answer);
        (round_trips, longest_wait)
    };

    println!(
        "--- node 2 answered {} heartbeats in {:?}: longest round trip {:?}, longest wait for an answer {:?}",
        round_trips.len(),
        WATCH,
        round_trips.iter().max(),
        longest_wait
    );
    {
        let interval = Duration::from_millis(HEARTBEAT_INTERVAL);
        assert!(
            round_trips.len() >= 5,
            "expect heartbeats to be answered while flushes are held, got {}",
            round_trips.len()
        );
        assert!(
            longest_wait < 5 * interval,
            "expect a heartbeat answer at least every {:?}, waited {:?}",
            5 * interval,
            longest_wait
        );
    }

    println!(
        "--- flush node 2's new appends at once, and wait for it to answer up to index {}",
        last_index
    );
    {
        log2.set_flush_delay(Duration::ZERO);
        raft1
            .wait(Some(Duration::from_secs(30)))
            .metrics(
                |m| matched_index(m, 2) >= Some(last_index),
                "node 2 answers the last write",
            )
            .await?;

        let append_took = TypeConfig::now() - slow_since;
        println!(
            "--- node 2 answered the writes {:?} after its flushes were slowed; each held flush took {:?}",
            append_took, FLUSH_DELAY
        );
        assert!(
            append_took >= FLUSH_DELAY,
            "expect the writes to wait for a held flush, took {:?}",
            append_took
        );
    }

    Ok(())
}
