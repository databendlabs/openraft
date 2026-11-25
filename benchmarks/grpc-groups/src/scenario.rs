use std::fmt;

/// What a run does once every group has a leader and the groups have settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Scenario {
    /// Measure one window, `measure`, without a fault.
    Steady,

    /// Measure a `baseline` window, kill node 1 with `SIGKILL`, measure the `failover` of the
    /// groups it led, then a `degraded` window without it.
    Crash,

    /// Measure a `baseline` window, stop node 1 with `SIGSTOP`, measure the `failover` of the
    /// groups it led and a `degraded` window without it, then resume it with `SIGCONT`, and
    /// measure its `rejoin` and an `after` window.
    Hang,
}

impl fmt::Display for Scenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scenario::Steady => write!(f, "steady"),
            Scenario::Crash => write!(f, "crash"),
            Scenario::Hang => write!(f, "hang"),
        }
    }
}
