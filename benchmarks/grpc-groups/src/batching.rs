/// How the nodes send their heartbeats: options of `run`, which passes them on to each node.
#[derive(Debug, Clone, Copy, Default, clap::Args)]
pub(crate) struct Batching {
    /// Send the AppendEntries requests without entries, mostly heartbeats, that a node's router
    /// gets within 1 ms to each target node in one call.
    #[arg(long)]
    pub(crate) batch_heartbeats: bool,

    /// Turn off the heartbeat timer of each group, with `enable_heartbeat`, and trigger the
    /// heartbeats of all groups of a node from one timer, so that they share a batch.
    #[arg(long)]
    pub(crate) node_heartbeat_timer: bool,
}

impl Batching {
    /// The flags that pass these options to a node process.
    pub(crate) fn flags(&self) -> Vec<&'static str> {
        let mut flags = Vec::new();
        if self.batch_heartbeats {
            flags.push("--batch-heartbeats");
        }
        if self.node_heartbeat_timer {
            flags.push("--node-heartbeat-timer");
        }
        flags
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Batching;

    #[derive(Parser)]
    struct Args {
        #[command(flatten)]
        batching: Batching,
    }

    #[test]
    fn test_flags_pass_the_options_on() {
        for flags in [vec![], vec!["--batch-heartbeats"], vec![
            "--batch-heartbeats",
            "--node-heartbeat-timer",
        ]] {
            let args = Args::parse_from(std::iter::once("node").chain(flags.iter().copied()));
            assert_eq!(flags, args.batching.flags());
        }
    }
}
