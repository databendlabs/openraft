//! Entry identities observed at append time, including entries later lost or purged.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::typ::Entry;
use crate::typ::LogId;
use crate::typ::NodeId;

#[derive(Default)]
pub(crate) struct EntryHistory {
    /// The entry under each log id, and every node that appended it.
    entries: BTreeMap<LogId, (Entry, BTreeSet<NodeId>)>,
    violations: Vec<String>,
}

impl EntryHistory {
    pub(crate) fn record(&mut self, node: NodeId, entry: &Entry) {
        let Some((recorded, holders)) = self.entries.get_mut(&entry.log_id) else {
            let holders = BTreeSet::from([node]);
            self.entries.insert(entry.log_id, (entry.clone(), holders));
            return;
        };
        if recorded == entry {
            holders.insert(node);
            return;
        }
        // Only this node appended the recorded entry, and it lost that entry
        // before any other node received it: no node can mistake one for the other.
        let only_on_this_node = holders.iter().all(|holder| *holder == node);
        if only_on_this_node {
            *recorded = entry.clone();
            return;
        }
        self.violations.push(format!(
            "LogIdReused: {} held by nodes {holders:?}: {:?}; now on n{node}: {:?}",
            entry.log_id, recorded.payload, entry.payload
        ));
    }

    pub(crate) fn drain_violations(&mut self) -> Vec<String> {
        std::mem::take(&mut self.violations)
    }
}

#[cfg(test)]
mod entry_history_test;
