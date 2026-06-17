//! Step-down watcher: a task that steps down a Leader that is removed from a committed
//! membership config.

use std::time::Duration;

use tracing::Instrument;

use crate::Config;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::StepDownPolicy;
use crate::async_runtime::MpscSender;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::core::ServerState;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::external_command::ExternalCommand;
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
/// It is an automation plugin outside the consensus kernel: it only sends the commands an
/// application could send manually, [`TriggerTransferLeader`] and [`RefreshServerState`], both of
/// which re-validate the state inside `RaftCore`. Therefore acting on stale or coalesced metrics
/// is harmless: an outdated command is just a no-op.
///
/// The task quits when `RaftCore` terminates.
///
/// [`removed_leader_step_down`]: crate::Config::removed_leader_step_down
/// [`TriggerTransferLeader`]: ExternalCommand::TriggerTransferLeader
/// [`RefreshServerState`]: ExternalCommand::RefreshServerState
pub(crate) struct StepDownWatcher<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    rx_server_metrics: WatchReceiverOf<C, RaftServerMetrics<C>>,

    /// The full metrics, read only when acting, to choose the transfer-leader target.
    rx_metrics: WatchReceiverOf<C, RaftMetrics<C>>,

    /// Weak sender to the `RaftCore` API channel: the watcher must not keep the channel alive.
    tx_api: MpscWeakSenderOf<C, RaftMsg<C, SD>>,

    /// The delay between the commit of the removing membership config and the transfer-leader.
    step_down_delay: Duration,

    /// The delay between the transfer-leader and reverting this node to a learner.
    transfer_wait: Duration,
}

impl<C, SD> StepDownWatcher<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
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
        tx_api: MpscWeakSenderOf<C, RaftMsg<C, SD>>,
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
        let watch_fut = this.watch_loop().instrument(span);
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
    /// - Wait for `step_down_delay`, then re-check the condition: it may no longer hold, e.g., a
    ///   newer membership config that contains this node is committed.
    /// - Transfer leadership to the most up-to-date voter.
    /// - Wait for `transfer_wait`, then revert this node to a learner. This is the guaranteed
    ///   fallback in case the transfer takes no effect; the command is fenced by the vote and the
    ///   membership config log id observed above, thus it is dropped if a new Leader is already
    ///   established or the membership config has changed.
    async fn try_step_down(&self) {
        // Clone the metrics out of the watch channel: the borrowed reference is not `Send`,
        // and holding it across an `await` would block the metrics updater.
        let server_metrics = self.rx_server_metrics.borrow_watched().clone();
        if !is_removed_leader(&server_metrics) {
            return;
        }

        C::sleep(self.step_down_delay).await;

        let server_metrics = self.rx_server_metrics.borrow_watched().clone();
        if !is_removed_leader(&server_metrics) {
            return;
        }

        let metrics = self.rx_metrics.borrow_watched().clone();
        let target = transfer_target(&metrics);

        if let Some(to) = target {
            tracing::info!("removed Leader steps down: transfer leadership to {}", to);
            self.send(ExternalCommand::TriggerTransferLeader { to }).await;
        }

        C::sleep(self.transfer_wait).await;

        // In the fence a `None` membership log id means skipping the membership check, thus it
        // must be `Some` here. It always holds: this node is a Leader, hence the cluster is
        // initialized, hence the membership config comes from a log entry that has a log id.
        debug_assert!(
            server_metrics.membership_config.log_id().is_some(),
            "the membership config of a Leader must have a log id"
        );

        let cmd = ExternalCommand::RefreshServerState {
            vote: Some(server_metrics.vote.clone()),
            membership_log_id: server_metrics.membership_config.log_id().clone(),
        };
        self.send(cmd).await;
    }

    /// Send an external command to `RaftCore`.
    ///
    /// A failure to upgrade the weak sender or to send means `RaftCore` is terminated. It is
    /// safe to ignore it here: the watch loop quits at the next `changed()` call in this case.
    async fn send(&self, cmd: ExternalCommand<C, SD>) {
        let Some(tx_api) = self.tx_api.upgrade() else {
            return;
        };

        let _ = tx_api.send(RaftMsg::ExternalCommand { cmd }).await;
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

/// Choose the transfer-leader target: the voter with the greatest matching log id, which needs
/// the least catching up.
fn transfer_target<C>(metrics: &RaftMetrics<C>) -> Option<C::NodeId>
where C: RaftTypeConfig {
    // Only a Leader publishes replication metrics.
    let replication = metrics.replication.as_ref()?;

    let membership = metrics.membership_config.membership();
    let matching = |id: &C::NodeId| replication.get(id).cloned().flatten();

    membership.voter_ids().max_by_key(matching)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use maplit::btreemap;
    use maplit::btreeset;

    use super::is_removed_leader;
    use super::transfer_target;
    use crate::Membership;
    use crate::core::ServerState;
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::metrics::RaftMetrics;
    use crate::metrics::RaftServerMetrics;
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
}
