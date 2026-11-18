use std::fmt;

use clap::ValueEnum;

use crate::Raft;

/// Which timers of every group stay on during the measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Mode {
    /// Keep heartbeats, elections and ticks on, as the default `Config` has them.
    Default,
    /// Turn elections off, and keep heartbeats and ticks on.
    NoElect,
    /// Turn heartbeats off, and keep elections and ticks on.
    NoHeartbeat,
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
            Mode::NoElect => runtime_config.elect(false),
            Mode::NoHeartbeat => runtime_config.heartbeat(false),
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

#[cfg(test)]
mod tests {
    use clap::ValueEnum;

    use super::Mode;

    #[test]
    fn test_reports_name_each_mode_as_the_command_line_does() {
        let names: Vec<String> = Mode::value_variants().iter().map(|mode| mode.to_string()).collect();

        let expected = ["default", "no-elect", "no-heartbeat", "no-heartbeat-elect", "no-tick"];
        assert_eq!(expected.map(String::from).to_vec(), names);
    }
}
