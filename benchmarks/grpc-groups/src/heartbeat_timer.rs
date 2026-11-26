use std::time::Duration;

use tokio::time::MissedTickBehavior;

use crate::Raft;

/// Trigger a heartbeat in each of `groups` every `interval`, until the node exits. Only a leader
/// sends heartbeats: a follower ignores the trigger.
///
/// With `--node-heartbeat-timer`, the groups have no heartbeat timer of their own, and this one
/// timer makes the heartbeats of all groups that the node leads reach the router at about the same
/// time, so that they can share a batch.
pub(crate) async fn trigger_heartbeats(groups: Vec<Raft>, interval: Duration) {
    let mut ticks = tokio::time::interval(interval);
    // A round that runs late is not made up for with rounds in quick succession.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        ticks.tick().await;
        for raft in &groups {
            // Only a member that has stopped fails the trigger, and it needs no heartbeat.
            raft.trigger().heartbeat().await.ok();
        }
    }
}
