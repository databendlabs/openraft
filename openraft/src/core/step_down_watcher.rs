//! Step-down watcher: a task that steps down a Leader that is removed from a committed
//! membership config.

use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use futures_util::future::Either;
use futures_util::future::select;
use tracing::Instrument;

use crate::Config;
use crate::RaftTypeConfig;
use crate::StepDownPolicy;
use crate::async_runtime::MpscSender;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::ServerState;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::leader_removal::LeaderRemoval;
use crate::membership::RaftMembershipExt;
use crate::metrics::RaftMetrics;
use crate::metrics::RaftServerMetrics;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::MpscWeakSenderOf;
use crate::type_config::alias::WatchReceiverOf;

/// Steps down a Leader that is removed from a committed membership config.
///
/// It watches the server metrics, which change only on server state transitions, such as when
/// the membership config that removes this Leader is committed. When this node is a Leader and
/// the committed membership config does not contain it, after the delay given by
/// [`removed_leader_step_down`] it transfers leadership to the most up-to-date voter (chosen
/// from a snapshot of the full metrics), then after another `heartbeat_interval` it reverts this
/// node to a learner, in case the transfer does not take effect.
///
/// It is an automation plugin outside the consensus kernel: it sends `RaftCore` two commands,
/// [`TriggerTransferLeader`] and [`RefreshServerState`], each fenced by the removal it observed:
/// the vote of this Leader and the log id of the membership config that removes it. `RaftCore`
/// re-validates a command when it handles it and drops it if the fence no longer matches, such as
/// after a membership change or a new vote that it handled first. The transfer also requires this
/// node to be a Leader whose effective membership config is committed and does not contain it.
/// Therefore acting on stale or coalesced metrics is harmless: an outdated command is a no-op.
///
/// The task quits as soon as `RaftCore` terminates, also while it sleeps or waits to send a
/// command, see [`until_closed()`].
///
/// [`removed_leader_step_down`]: crate::Config::removed_leader_step_down
/// [`TriggerTransferLeader`]: ExternalCommand::TriggerTransferLeader
/// [`RefreshServerState`]: ExternalCommand::RefreshServerState
pub(crate) struct StepDownWatcher<C>
where C: RaftTypeConfig
{
    rx_server_metrics: WatchReceiverOf<C, RaftServerMetrics<C>>,

    /// The full metrics, read only when acting, to choose the transfer-leader target.
    rx_metrics: WatchReceiverOf<C, RaftMetrics<C>>,

    /// Weak sender to the `RaftCore` API channel: the watcher must not keep the channel alive.
    tx_api: MpscWeakSenderOf<C, RaftMsg<C>>,

    /// The delay between the commit of the removing membership config and the transfer-leader.
    step_down_delay: Duration,

    /// The delay between the transfer-leader and reverting this node to a learner.
    transfer_wait: Duration,
}

impl<C> StepDownWatcher<C>
where C: RaftTypeConfig
{
    /// Spawn the watcher task.
    ///
    /// It does nothing if the automated step down is disabled,
    /// i.e., [`removed_leader_step_down`] is [`StepDownPolicy::Never`].
    ///
    /// [`removed_leader_step_down`]: crate::Config::removed_leader_step_down
    pub(crate) fn spawn(
        rx_server_metrics: WatchReceiverOf<C, RaftServerMetrics<C>>,
        rx_metrics: WatchReceiverOf<C, RaftMetrics<C>>,
        tx_api: MpscWeakSenderOf<C, RaftMsg<C>>,
        config: &Config,
    ) {
        let delay = match config.removed_leader_step_down {
            StepDownPolicy::Never => return,
            StepDownPolicy::After(ms) => ms,
        };

        let this = Self {
            rx_server_metrics,
            rx_metrics,
            tx_api,
            step_down_delay: Duration::from_millis(delay),
            transfer_wait: Duration::from_millis(config.heartbeat_interval),
        };

        let span = tracing::debug_span!("step_down_watcher");
        let rx_closed = this.rx_server_metrics.clone();
        let watch_fut = until_closed::<C, _>(rx_closed, this.watch_loop()).instrument(span);
        let _join_handle = C::spawn(watch_fut);
    }

    async fn watch_loop(mut self) {
        loop {
            self.try_step_down().await;

            let recv_res = self.rx_server_metrics.changed().await;
            if recv_res.is_err() {
                tracing::info!("RaftCore terminated, quit step_down_watcher");
                return;
            }
        }
    }

    /// Step down this node if it is a Leader that is removed from a committed membership config:
    ///
    /// - Wait for `step_down_delay` after the removal is observed. The delay belongs to that
    ///   removal: the vote and the committed membership config that removes this node. It ends
    ///   early when this node is no longer a removed Leader, such as when a newer membership config
    ///   that contains it is committed, and it starts again when the removal is another one, such
    ///   as a second removal after a readmission, or the same config under a new vote. Other
    ///   metrics updates leave the deadline as it is.
    /// - Transfer leadership to the most up-to-date voter. The command is fenced by the removal:
    ///   `RaftCore` drops it unless this node is still a Leader with the vote of the removal, whose
    ///   effective membership config is the removing one, committed, and without this node.
    /// - Wait for `transfer_wait`, then revert this node to a learner. This is the guaranteed
    ///   fallback in case the transfer takes no effect; the command is fenced by the vote and the
    ///   membership config log id of the removal, thus it is dropped if a new Leader is already
    ///   established or the membership config has changed.
    async fn try_step_down(&mut self) {
        // Clone the metrics out of the watch channel: the borrowed reference is not `Send`,
        // and holding it across an `await` would block the metrics updater.
        let mut server_metrics = self.rx_server_metrics.borrow_watched().clone();
        if !is_removed_leader(&server_metrics) {
            return;
        }

        let mut deadline = C::now() + self.step_down_delay;
        loop {
            let closed = {
                let sleep = pin!(C::sleep_until(deadline));
                let changed = pin!(self.rx_server_metrics.changed());
                matches!(select(sleep, changed).await, Either::Right((Err(_), _)))
            };
            if closed {
                tracing::info!("RaftCore terminated during the step-down delay, quit step_down_watcher");
                return;
            }

            // Re-check after any wake-up: a change and the deadline may come at once.
            let latest = self.rx_server_metrics.borrow_watched().clone();
            if !is_removed_leader(&latest) {
                return;
            }
            if !is_same_removal(&server_metrics, &latest) {
                tracing::info!("another removal of this Leader is observed, restart the step-down delay");
                server_metrics = latest;
                deadline = C::now() + self.step_down_delay;
                continue;
            }
            if C::now() >= deadline {
                break;
            }
        }

        // The removal fences both commands. In the fence of the fallback a `None` membership log
        // id means skipping the membership check, thus it must be `Some` here. It always holds:
        // this node is a Leader, hence the cluster is initialized, hence the membership config
        // comes from a log entry that has a log id.
        debug_assert!(
            server_metrics.membership_config.log_id().is_some(),
            "the membership config of a Leader must have a log id"
        );
        let removal = LeaderRemoval {
            vote: server_metrics.vote.clone(),
            membership_log_id: server_metrics.membership_config.log_id().clone(),
        };

        let metrics = self.rx_metrics.borrow_watched().clone();
        let target = transfer_target(&metrics);

        if let Some(to) = target {
            tracing::info!("removed Leader steps down: transfer leadership to {}", to);
            let cmd = ExternalCommand::TriggerTransferLeader {
                to,
                removal: Some(removal.clone()),
            };
            self.send(cmd).await;
        }

        C::sleep(self.transfer_wait).await;

        let cmd = ExternalCommand::RefreshServerState {
            vote: Some(removal.vote),
            membership_log_id: removal.membership_log_id,
        };
        self.send(cmd).await;
    }

    /// Send an external command to `RaftCore`.
    ///
    /// A failure to upgrade the weak sender or to send means `RaftCore` is terminated. It is
    /// safe to ignore it here: the task quits once `RaftCore` drops the server metrics sender.
    async fn send(&self, cmd: ExternalCommand<C>) {
        let Some(tx_api) = self.tx_api.upgrade() else {
            return;
        };

        let _ = tx_api.send(RaftMsg::ExternalCommand { cmd }).await;
    }
}

/// Run `work` until it returns or `RaftCore` terminates, whichever comes first.
///
/// `RaftCore` terminates when it drops the server metrics sender. `rx` is a receiver of its own, so
/// the metrics updates it takes stay unseen for `work`, and it waits until `changed()` reports the
/// closure, which comes after a last value it has not seen. Dropping `work` then cancels a pending
/// sleep or send, and releases what `work` holds, such as its metrics receivers.
async fn until_closed<C, F>(mut rx: WatchReceiverOf<C, RaftServerMetrics<C>>, work: F)
where
    C: RaftTypeConfig,
    F: Future<Output = ()>,
{
    let closed = async move { while rx.changed().await.is_ok() {} };
    if let Either::Right(_) = select(pin!(work), pin!(closed)).await {
        tracing::info!("RaftCore terminated, quit step_down_watcher");
    }
}

/// Decide whether this node is a Leader that is removed from a committed membership config.
///
/// The decision is level-based: it is derived from a single metrics snapshot, so skipped or
/// stale snapshots do not affect correctness.
fn is_removed_leader<C>(metrics: &RaftServerMetrics<C>) -> bool
where C: RaftTypeConfig {
    if metrics.state != ServerState::Leader {
        return false;
    }

    // A Leader that is still in the membership config, as a voter or a learner, keeps leading.
    if metrics.membership_config.membership().contains(&metrics.id) {
        return false;
    }

    // The Leader keeps leading until the membership config that removes it is committed,
    // because it is the one responsible for replicating this config.
    // The committed membership config equals the effective one iff the latter is committed.
    metrics.committed_membership_config == metrics.membership_config
}

/// Decide whether two snapshots observe the same removal of this Leader: the same vote, and the
/// same committed membership config, which removes it.
fn is_same_removal<C>(a: &RaftServerMetrics<C>, b: &RaftServerMetrics<C>) -> bool
where C: RaftTypeConfig {
    a.vote == b.vote && a.committed_membership_config.log_id() == b.committed_membership_config.log_id()
}

/// Choose the transfer-leader target: the voter with the greatest matching log id, which needs
/// the least catching up.
fn transfer_target<C>(metrics: &RaftMetrics<C>) -> Option<C::NodeId>
where C: RaftTypeConfig {
    // Only a Leader publishes replication metrics.
    let replication = metrics.replication.as_ref()?;

    let matching = |id: &C::NodeId| replication.get(id).cloned().flatten();

    metrics.membership_config.voter_ids().max_by_key(matching)
}

#[cfg(test)]
mod step_down_watcher_test;
