use std::fmt;

use crate::scenario::Scenario;

/// The fields that begin each line of a run: its scenario, the phase, and its size.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Phase {
    pub(crate) scenario: Scenario,
    pub(crate) name: &'static str,
    pub(crate) nodes: u64,
    pub(crate) groups: u64,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "scenario={}\tphase={}\tnodes={}\tgroups={}",
            self.scenario, self.name, self.nodes, self.groups
        )
    }
}
