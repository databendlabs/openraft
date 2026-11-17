//! Updatable config for a raft runtime.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use crate::Config;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchSender;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::WatchReceiverOf;
use crate::type_config::alias::WatchSenderOf;

/// Updatable config for a raft runtime.
pub(crate) struct RuntimeConfig<C>
where C: RaftTypeConfig
{
    /// A watch channel, so that a disabled tick loop waits for a change instead of polling.
    pub(crate) enable_tick: Mutex<WatchSenderOf<C, bool>>,
    pub(crate) enable_heartbeat: AtomicBool,
    pub(crate) enable_elect: AtomicBool,
    pub(crate) enable_pre_vote: AtomicBool,

    /// Counts the changes of the switches above, so that `RaftCore` wakes up to apply a change.
    changes: Mutex<WatchSenderOf<C, u64>>,
}

impl<C> RuntimeConfig<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(config: &Config) -> Self {
        let (enable_tick, _rx) = C::watch_channel(config.enable_tick);
        let enable_tick = Mutex::new(enable_tick);
        let (changes, _rx) = C::watch_channel(0);
        Self {
            enable_tick,
            enable_heartbeat: AtomicBool::from(config.enable_heartbeat),
            enable_elect: AtomicBool::from(config.enable_elect),
            enable_pre_vote: AtomicBool::from(config.get_enable_pre_vote()),
            changes: Mutex::new(changes),
        }
    }

    /// Return a receiver that becomes ready when a switch changes.
    pub(crate) fn subscribe_changes(&self) -> WatchReceiverOf<C, u64> {
        self.changes.lock().unwrap().subscribe()
    }

    /// Set `switch`, one of the flags of this config, to `enabled`, and wake `RaftCore` if its
    /// value changed.
    pub(crate) fn set_switch(&self, switch: &AtomicBool, enabled: bool) {
        let previous = switch.swap(enabled, Ordering::Relaxed);
        if previous != enabled {
            self.notify_changed();
        }
    }

    /// Wake `RaftCore`, which applies the switches when it next runs a round.
    pub(crate) fn notify_changed(&self) {
        self.changes.lock().unwrap().send_if_modified(|changes| {
            *changes += 1;
            true
        });
    }
}
