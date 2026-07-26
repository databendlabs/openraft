//! Metrics, progress watchers, and notifications about state changes.

use std::future::Future;
use std::time::Duration;

use openraft_macros::since;

use crate::OptionalSend;
use crate::Raft;
use crate::RaftTypeConfig;
use crate::core::io_flush_tracking::AppliedProgress;
use crate::core::io_flush_tracking::CommitProgress;
use crate::core::io_flush_tracking::LogProgress;
use crate::core::io_flush_tracking::SnapshotProgress;
use crate::core::io_flush_tracking::VoteProgress;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::Wait;
use crate::metrics::WaitError;
use crate::raft::watch_handle::WatchChangeHandle;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::NodeIdOf;
use crate::type_config::alias::WatchReceiverOf;
use crate::vote::Vote;
use crate::vote::leader_id::raft_leader_id::RaftLeaderId;

/// Implement the observation API: point-in-time metrics, watch channels that report progress as
/// it advances, and helpers that wait for a condition to hold.
impl<C, SM> Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Get a handle to the metrics channel.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Get current metrics
    /// let metrics = raft.metrics().borrow_watched().clone();
    /// println!("Current leader: {:?}", metrics.current_leader);
    /// println!("Current term: {}", metrics.current_term);
    /// ```
    pub fn metrics(&self) -> WatchReceiverOf<C, RaftMetrics<C>> {
        self.inner.rx_metrics.clone()
    }

    /// Get a handle to the data metrics channel.
    pub fn data_metrics(&self) -> WatchReceiverOf<C, RaftDataMetrics<C>> {
        self.inner.rx_data_metrics.clone()
    }

    /// Get a handle to the server metrics channel.
    pub fn server_metrics(&self) -> WatchReceiverOf<C, RaftServerMetrics<C>> {
        self.inner.rx_server_metrics.clone()
    }

    /// Get a handle to watch log I/O flush progress.
    ///
    /// Tracks when log entries and votes are durably written to storage.
    /// Updated on every I/O completion (vote saves and log appends).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut log_progress = raft.watch_log_progress();
    ///
    /// // Wait for a specific log entry to be flushed
    /// let target = Some(FlushPoint::new(
    ///     Vote::new_committed(2, node_id),
    ///     Some(LogId::new(LeaderId::new(2, node_id), 100))
    /// ));
    /// log_progress.wait_until_ge(&target).await?;
    /// ```
    #[since(version = "0.10.0")]
    #[must_use = "progress handle should be stored to track I/O progress"]
    pub fn watch_log_progress(&self) -> LogProgress<C> {
        self.inner.progress_watcher.log_progress()
    }

    /// Get a handle to watch vote I/O flush progress.
    ///
    /// Tracks when votes (leadership changes) are durably written to storage.
    /// Updated only when the vote changes (new term or leader), not on every log append.
    ///
    /// Use this when you only care about leadership changes, not specific log entries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut vote_progress = raft.watch_vote_progress();
    ///
    /// // Wait for term 2 to be persisted
    /// let target = Some(Vote::new_committed(2, 0));
    /// vote_progress.wait_until_ge(&target).await?;
    /// ```
    #[since(version = "0.10.0")]
    #[must_use = "progress handle should be stored to track vote progress"]
    pub fn watch_vote_progress(&self) -> VoteProgress<C> {
        self.inner.progress_watcher.vote_progress()
    }

    /// Get a handle to watch commit log progress.
    ///
    /// Tracks when committed logs advance(persisted on a quorum and the last-log is proposed by the
    /// leader). Updated whenever the committed cursor moves forward.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut commit_progress = raft.watch_commit_progress();
    ///
    /// // Wait until log index 42 is committed
    /// let target = Some(LogId::new(LeaderId::new(2, node_id), 42));
    /// commit_progress.wait_until_ge(&target).await?;
    /// ```
    #[since(version = "0.10.0")]
    #[must_use = "progress handle should be stored to track commit progress"]
    pub fn watch_commit_progress(&self) -> CommitProgress<C> {
        self.inner.progress_watcher.commit_progress()
    }

    /// Get a handle to watch snapshot persistence progress.
    ///
    /// Tracks when snapshots are persisted to storage.
    /// Updated whenever a snapshot is built or installed and persisted.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut snapshot_progress = raft.watch_snapshot_progress();
    ///
    /// // Wait until snapshot covering log index 100 is persisted
    /// let target = Some(LogId::new(LeaderId::new(2, node_id), 100));
    /// snapshot_progress.wait_until_ge(&target).await?;
    /// ```
    #[since(version = "0.10.0")]
    #[must_use = "progress handle should be stored to track snapshot progress"]
    pub fn watch_snapshot_progress(&self) -> SnapshotProgress<C> {
        self.inner.progress_watcher.snapshot_progress()
    }

    /// Get a handle to watch applied log progress.
    ///
    /// Tracks when logs are applied to the state machine.
    /// Updated whenever the last applied log id advances.
    ///
    /// # Note
    ///
    /// If the state machine does not persist the applied state immediately, the watcher
    /// may observe duplicate events when the server restarts and re-applies log entries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut apply_progress = raft.watch_apply_progress();
    ///
    /// // Wait until log index 42 is applied
    /// let target = Some(LogId::new(LeaderId::new(2, node_id), 42));
    /// apply_progress.wait_until_ge(&target).await?;
    /// ```
    #[since(version = "0.10.0")]
    #[must_use = "progress handle should be stored to track applied progress"]
    pub fn watch_apply_progress(&self) -> AppliedProgress<C> {
        self.inner.progress_watcher.apply_progress()
    }

    /// Watch for any leader changes in the cluster and invoke callback on each change.
    ///
    /// Returns a [`WatchChangeHandle`] that must be held to keep watching.
    /// If the handle is dropped or [`WatchChangeHandle::close()`] is called,
    /// the background task will be terminated and the callback will no longer be invoked.
    ///
    /// The callback receives:
    /// - `old`: The previous leader state `(leader_id, committed)`, or `None` on the first callback
    /// - `new`: The current leader state `(leader_id, committed)`
    ///
    /// This fires on ANY leader change in the cluster, not just when this node's leadership
    /// changes. For a simpler API that only fires when THIS node becomes or loses leadership,
    /// see [`on_leader_change()`].
    ///
    /// # Note on Start/Stop Service Pattern
    ///
    /// If you use this API to start/stop services based on leadership, be aware that
    /// consecutive callbacks may show the same node as leader with different Terms
    /// (e.g., Term 1 → Term 2). The simple `if is_leader { start } else { stop }` pattern
    /// could call `start` twice without an intervening `stop`.
    ///
    /// For the start/stop service pattern, prefer [`on_leader_change()`] which guarantees
    /// alternating `start`/`stop` callbacks.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let my_node_id = 1;
    ///
    /// let mut handle = raft.on_cluster_leader_change(move |_old, (leader_id, committed)| {
    ///     let is_leader = leader_id.node_id == my_node_id && committed;
    ///
    ///     async move {
    ///         if is_leader {
    ///             // This node just became the committed leader
    ///             // Start leader-only services (e.g., cron jobs, cache warming)
    ///             start_leader_services().await;
    ///         } else {
    ///             // This node is no longer the leader
    ///             // Stop leader-only services to avoid duplicate work
    ///             stop_leader_services().await;
    ///         }
    ///     }
    /// });
    ///
    /// // Later, stop watching
    /// handle.close().await;
    /// ```
    ///
    /// [`on_leader_change()`]: Self::on_leader_change
    #[since(version = "0.10.0")]
    #[must_use = "handle must be held to keep the watch task running"]
    pub fn on_cluster_leader_change<F, Fut>(&self, mut callback: F) -> WatchChangeHandle<C>
    where
        F: FnMut(Option<(C::LeaderId, bool)>, (C::LeaderId, bool)) -> Fut + OptionalSend + 'static,
        Fut: Future<Output = ()> + OptionalSend + 'static,
    {
        let mut prev_vote: Option<Vote<C::LeaderId>> = None;

        self.watch_vote_change(move |new_vote, _my_node_id| {
            let old_leader = prev_vote.as_ref().map(|v| v.leader_id().clone());
            let new_leader = new_vote.leader_id().clone();

            // Only call callback if leader_id actually changed
            let fut = if old_leader.as_ref() != Some(&new_leader) {
                let old_state = prev_vote.as_ref().map(|v| (v.leader_id().clone(), v.is_committed()));
                let new_state = (new_vote.leader_id().clone(), new_vote.is_committed());
                Some(callback(old_state, new_state))
            } else {
                None
            };
            prev_vote = Some(new_vote);

            async move {
                if let Some(f) = fut {
                    f.await;
                }
            }
        })
    }

    /// Register callbacks for when this node becomes or stops being the committed leader.
    ///
    /// Returns a [`WatchChangeHandle`] that must be held to keep watching.
    /// If the handle is dropped or [`WatchChangeHandle::close()`] is called,
    /// the background task will be terminated and the callbacks will no longer be invoked.
    ///
    /// Unlike [`on_cluster_leader_change()`] which fires on any leader change in the cluster,
    /// this method only fires when THIS node becomes or stops being the leader.
    ///
    /// - `start`: Called when this node becomes the leader (committed, quorum-acknowledged)
    /// - `stop`: Called when this node is no longer the leader (another node becomes leader)
    ///
    /// # Callback Guarantees
    ///
    /// The `start` and `stop` callbacks are guaranteed to be called in alternating order:
    /// `start` → `stop` → `start` → `stop` → ...
    ///
    /// Even if a node transitions directly from leader in Term 1 to leader in Term 2,
    /// `stop` will be called with the old `leader_id` before `start` is called with the
    /// new `leader_id`. This ensures proper resource cleanup between leadership terms.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let handle = raft.on_leader_change(
    ///     |leader_id| async move {
    ///         println!("Became leader: {:?}", leader_id);
    ///         start_leader_services().await;
    ///     },
    ///     |old_leader| async move {
    ///         println!("Stopped leading: {:?}", old_leader);
    ///         stop_leader_services().await;
    ///     },
    /// );
    ///
    /// // Later, stop watching
    /// handle.close().await;
    /// ```
    ///
    /// [`on_cluster_leader_change()`]: Self::on_cluster_leader_change
    #[since(version = "0.10.0")]
    #[must_use = "handle must be held to keep the watch task running"]
    pub fn on_leader_change<F1, F2, Fut1, Fut2>(&self, start: F1, stop: F2) -> WatchChangeHandle<C>
    where
        F1: Fn(C::LeaderId) -> Fut1 + OptionalSend + 'static,
        F2: Fn(C::LeaderId) -> Fut2 + OptionalSend + 'static,
        Fut1: Future<Output = ()> + OptionalSend + 'static,
        Fut2: Future<Output = ()> + OptionalSend + 'static,
    {
        let mut prev_leader_id = None;

        self.watch_vote_change(move |vote, my_node_id| {
            let leader_id = vote.leader_id().clone();

            // Fire `start` when THIS node becomes committed leader
            // and it's a new leadership (different from current)
            #[allow(clippy::collapsible_else_if)]
            let (stop_fut, start_fut) = if leader_id.node_id() == my_node_id {
                if vote.is_committed() && prev_leader_id.as_ref() != Some(&leader_id) {
                    // Call stop first if transitioning from one leadership to another
                    // (e.g., Term 1 leader -> Term 2 leader)
                    // This guarantees alternating start/stop calls.
                    let stop_fut = prev_leader_id.take().map(&stop);
                    let start_fut = Some(start(leader_id.clone()));
                    prev_leader_id = Some(leader_id);
                    (stop_fut, start_fut)
                } else {
                    (None, None)
                }
            } else {
                let stop_fut = prev_leader_id.take().map(&stop);
                (stop_fut, None)
            };

            async move {
                if let Some(f) = stop_fut {
                    f.await;
                }
                if let Some(f) = start_fut {
                    f.await;
                }
            }
        })
    }

    /// Spawn a task that watches vote changes and invokes async callback on each change.
    ///
    /// This is an internal helper used by [`Self::on_leader_change()`] and
    /// [`Self::on_cluster_leader_change()`].
    ///
    /// The callback returns a future that will be awaited before processing
    /// the next vote change.
    fn watch_vote_change<F, Fut>(&self, mut callback: F) -> WatchChangeHandle<C>
    where
        F: FnMut(Vote<C::LeaderId>, &NodeIdOf<C>) -> Fut + OptionalSend + 'static,
        Fut: Future<Output = ()> + OptionalSend + 'static,
    {
        use futures_util::FutureExt;

        let my_node_id = self.inner.id().clone();
        let mut vote_progress = self.watch_vote_progress();
        let (cancel_tx, cancel_rx) = C::oneshot::<()>();

        let handle = C::spawn(async move {
            let mut cancel_rx = cancel_rx.fuse();

            loop {
                futures_util::select! {
                    _ = cancel_rx => break,
                    res = vote_progress.changed().fuse() => {
                        if res.is_err() {
                            break;
                        }
                        let Some(vote) = vote_progress.get() else {
                            continue;
                        };

                        callback(vote, &my_node_id).await;
                    }
                }
            }
        });

        WatchChangeHandle {
            cancel_tx: Some(cancel_tx),
            join_handle: Some(handle),
        }
    }

    /// Get a handle to wait for the metrics to satisfy some condition.
    ///
    ///
    /// If `timeout` is `None`, then it will wait forever(10 years).
    /// If `timeout` is `Some`, then it will wait for the specified duration.
    ///
    /// ```ignore
    /// # use std::time::Duration;
    /// # use openraft::{State, Raft};
    ///
    /// let timeout = Duration::from_millis(200);
    ///
    /// // wait for raft log-3 to be received and applied:
    /// r.wait(Some(timeout)).log(Some(3), "log").await?;
    ///
    /// // wait for ever for raft node's current leader to become 3:
    /// r.wait(None).current_leader(2, "wait for leader").await?;
    ///
    /// // wait for raft state to become a follower
    /// r.wait(None).state(State::Follower, "state").await?;
    /// ```
    pub fn wait(&self, timeout: Option<Duration>) -> Wait<C> {
        self.inner.wait(timeout)
    }

    /// Wait for this node's state machine to recover to at least the state it had before restart.
    ///
    /// Call this **once**, right after the node is created, before it starts serving requests. It
    /// is a one-time step for recovering across a restart, **not** a per-read primitive: it does
    /// not make later reads linearizable — use [`ensure_linearizable`](Self::ensure_linearizable)
    /// for each linearizable read instead. (Once recovered, the node stays recovered, so calling it
    /// again is a no-op that returns as soon as the current cluster commit has been applied.)
    ///
    /// The method waits in two phases, together bounded by `timeout`:
    /// 1. until `cluster_committed` is perceived as covering this node's current durable log tail
    ///    (the cluster commit is re-established for everything this node may have applied),
    /// 2. until the state machine has applied up to that cluster commit.
    ///
    /// The target is pinned to the first `cluster_committed` that covers this node's current
    /// durable log tail. If the cluster advanced while this node was down, that target may exceed
    /// the node's own pre-restart applied index — which still satisfies "at least the same state".
    ///
    /// It requires a reachable, functioning cluster: if no cluster commit is re-established (no
    /// leader, lost quorum, or this node is isolated), the method returns [`WaitError::Timeout`].
    /// If `timeout` is `None` it waits forever (see [`wait`](Self::wait)).
    ///
    /// # Why this works
    ///
    /// Perceiving a `cluster_committed` that covers this node's durable log tail means the current
    /// leader has either committed the local tail this node may have applied before restart, or has
    /// first replaced any uncommitted tail with the leader's log. Applying up to that commit
    /// restores every durable log entry that could have contributed to the pre-restart state. This
    /// is the same `read_log_id` argument that makes linearizable reads safe: see [the read
    /// protocol](crate::docs::protocol::read) for the read-safety argument and [the commit
    /// protocol](crate::docs::protocol::commit) for why `cluster_committed` is never a value
    /// restored from storage. A node that did not persist `committed` can use this in place of, or
    /// together with, saving it (see [log pointers](crate::docs::data::log_pointers)).
    ///
    /// # Limitation: a follower may still recover a stale state
    ///
    /// The soundness above is bounded by what this node made **durable**. A follower may apply an
    /// entry the cluster has committed before it flushes that entry locally — apply is gated on the
    /// cluster commit, not on the local flush (the invariant is `applied ≤ committed ≤ submitted`,
    /// not `applied ≤ flushed`; see [log pointers](crate::docs::data::log_pointers)). If the node
    /// restarts while such an entry is applied but still unflushed, and its applied state was not
    /// otherwise persisted, the restart reverts three tracking points at once: the durable log tail
    /// falls back to `flushed`, the state machine falls back to an earlier applied state, and
    /// `last_log_index` now describes the shorter log.
    ///
    /// Because the first-phase wait is `cluster_committed >= last_log_index` and `last_log_index`
    /// has itself reverted, a stale `cluster_committed` covering only the shrunken log satisfies
    /// it. `wait_for_recovery` can then return having recovered a state older than the one this
    /// node served immediately before the restart. The node is not stuck — the leader
    /// re-replicates the lost tail and it catches up — but across that window the method no
    /// longer pins recovery to the pre-restart state.
    ///
    /// This can only happen while serving reads as a follower. A leader holds every committed entry
    /// and never applies below its commit, so a read taken on the leader through
    /// [`ensure_linearizable`](Self::ensure_linearizable) cannot revert to an earlier state. When a
    /// read must be guaranteed fresh across a restart, serve it on the leader.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // On startup, block until the state machine has recovered the committed state before
    /// // serving reads:
    /// raft.wait_for_recovery(Some(Duration::from_secs(5))).await?;
    /// ```
    #[since(version = "0.10.0")]
    pub async fn wait_for_recovery(&self, timeout: Option<Duration>) -> Result<RaftMetrics<C>, WaitError> {
        let start = C::now();

        // Phase 1: wait until the cluster commit is re-established for this node's durable log
        // tail. A follower may first hear an older leader_commit, which is not enough to recover
        // everything it had applied before restart.
        let metrics = self
            .wait(timeout)
            .metrics(
                |m| {
                    let cluster_committed = m.cluster_committed.as_ref().map(|x| x.index());
                    cluster_committed.is_some() && cluster_committed >= m.last_log_index
                },
                "wait_for_recovery: cluster commit covers local log",
            )
            .await?;

        let target = metrics.cluster_committed.as_ref().map(|x| x.index());

        // Phase 2: wait until the state machine has applied up to that cluster commit, within the
        // time remaining from the original budget.
        let remaining = timeout.map(|t| t.saturating_sub(C::now() - start));
        self.wait(remaining)
            .applied_index_at_least(target, "wait_for_recovery: state machine applied cluster commit")
            .await
    }
}
