use std::time::Duration;

use crate::Config;
use crate::RaftTypeConfig;
use crate::engine::time_state;
use crate::type_config::alias::AsyncRuntimeOf;

/// Config for Engine
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct EngineConfig<C: RaftTypeConfig> {
    /// The id of this node.
    pub(crate) id: C::NodeId,

    /// The maximum number of applied logs to keep before purging.
    pub(crate) max_in_snapshot_log_to_keep: u64,

    /// The minimal number of applied logs to purge in a batch.
    pub(crate) purge_batch_size: u64,

    /// The maximum number of entries per payload allowed to be transmitted during replication
    pub(crate) max_payload_entries: u64,

    pub(crate) allow_log_reversion: bool,

    pub(crate) timer_config: time_state::Config,
}

impl<C> EngineConfig<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(id: C::NodeId, config: &Config) -> Self {
        let election_timeout = Duration::from_millis(config.new_rand_election_timeout::<AsyncRuntimeOf<C>>());
        Self {
            id,
            max_in_snapshot_log_to_keep: config.max_in_snapshot_log_to_keep,
            purge_batch_size: config.purge_batch_size,
            max_payload_entries: config.max_payload_entries,
            allow_log_reversion: config.get_allow_log_reversion(),

            timer_config: time_state::Config {
                election_timeout,
                smaller_log_timeout: Duration::from_millis(config.election_timeout_max * 2),
                leader_lease: Duration::from_millis(config.election_timeout_max),
            },
        }
    }

    #[allow(dead_code)]
    pub(crate) fn new_default(id: C::NodeId) -> Self {
        Self {
            id,
            max_in_snapshot_log_to_keep: 1000,
            purge_batch_size: 256,
            max_payload_entries: 300,
            allow_log_reversion: false,
            timer_config: time_state::Config::default(),
        }
    }
}
