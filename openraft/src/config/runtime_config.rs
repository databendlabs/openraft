//! Updatable config for a raft runtime.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::Config;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchSender;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::WatchSenderOf;

/// Updatable config for a raft runtime.
pub(crate) struct RuntimeConfig<C>
where C: RaftTypeConfig
{
    /// A watch channel, so that a disabled tick loop waits for a change instead of polling.
    pub(crate) enable_tick: Mutex<WatchSenderOf<C, bool>>,
    pub(crate) enable_heartbeat: AtomicBool,
    /// A watch channel, so that a disabled election deadline notifier waits for a change, and
    /// fires at once for a due election when it is enabled again.
    pub(crate) enable_elect: Mutex<WatchSenderOf<C, bool>>,
    pub(crate) enable_pre_vote: AtomicBool,
}

impl<C> RuntimeConfig<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(config: &Config) -> Self {
        let (enable_tick, _rx) = C::watch_channel(config.enable_tick);
        let (enable_elect, _rx) = C::watch_channel(config.enable_elect);
        Self {
            enable_tick: Mutex::new(enable_tick),
            enable_heartbeat: AtomicBool::from(config.enable_heartbeat),
            enable_elect: Mutex::new(enable_elect),
            enable_pre_vote: AtomicBool::from(config.get_enable_pre_vote()),
        }
    }

    /// Whether automatic elections are enabled.
    pub(crate) fn is_elect_enabled(&self) -> bool {
        let enable_elect = self.enable_elect.lock().unwrap();
        *enable_elect.borrow_watched()
    }
}
