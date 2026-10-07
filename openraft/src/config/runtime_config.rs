//! Updatable config for a raft runtime.

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use crate::Config;
use crate::RaftTypeConfig;
use crate::type_config::TypeConfigExt;
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
}

impl<C> RuntimeConfig<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(config: &Config) -> Self {
        let (enable_tick, _rx) = C::watch_channel(config.enable_tick);
        let enable_tick = Mutex::new(enable_tick);
        Self {
            enable_tick,
            enable_heartbeat: AtomicBool::from(config.enable_heartbeat),
            enable_elect: AtomicBool::from(config.enable_elect),
            enable_pre_vote: AtomicBool::from(config.get_enable_pre_vote()),
        }
    }
}
