use std::fmt;

use clap::ValueEnum;

/// What stands in for the two followers of each group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Followers {
    /// Stubs in the router that grant every vote and acknowledge every AppendEntries request, so
    /// the numbers cover the leader members only.
    Stub,
    /// Real members in this process, so the numbers cover whole groups.
    Local,
}

impl fmt::Display for Followers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().expect("no variant is skipped");
        f.write_str(value.get_name())
    }
}
