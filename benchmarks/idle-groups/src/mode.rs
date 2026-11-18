use std::fmt;

use clap::ValueEnum;

use crate::Raft;

/// Which timers of every group stay on during the measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Mode {
    /// Keep heartbeats, elections and ticks on, as the default `Config` has them.
    Default,
    /// Turn heartbeats and elections off, and keep ticks on.
    NoHeartbeatElect,
    /// Turn ticks off.
    NoTick,
}

impl Mode {
    /// Switch the timers of `raft` as this mode says.
    pub(crate) fn apply(self, raft: &Raft) {
        let runtime_config = raft.runtime_config();
        match self {
            Mode::Default => {}
            Mode::NoHeartbeatElect => {
                runtime_config.heartbeat(false);
                runtime_config.elect(false);
            }
            Mode::NoTick => runtime_config.tick(false),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().expect("no mode is skipped");
        f.write_str(value.get_name())
    }
}
