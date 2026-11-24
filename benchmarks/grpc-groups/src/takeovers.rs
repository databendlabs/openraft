use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;

use crate::Raft;
use crate::clock;
use crate::protobuf as pb;
use crate::ready::is_ready_leader;

/// Records when the members of this node take over the groups whose leader failed.
#[derive(Clone, Default)]
pub(crate) struct Takeovers {
    /// The takeover of each group, by group id, once this node's member became its leader.
    records: Arc<Mutex<BTreeMap<u64, pb::Takeover>>>,
}

impl Takeovers {
    /// Watch the members of `groups` with the ids `group_ids`, for at most `limit`: record when one
    /// becomes leader, and when it then commits an entry of its term.
    pub(crate) fn arm(&self, groups: &[Raft], group_ids: &[u64], limit: Duration) -> anyhow::Result<()> {
        for &group_id in group_ids {
            let index = usize::try_from(group_id)?;
            let Some(raft) = groups.get(index).cloned() else {
                anyhow::bail!("no group {group_id}");
            };
            let records = self.records.clone();
            tokio::spawn(watch_takeover(raft, group_id, limit, records));
        }
        Ok(())
    }

    /// Return the takeovers recorded so far.
    pub(crate) fn recorded(&self) -> pb::Takeovers {
        let records = self.records.lock().expect("no thread panics while it holds the lock");
        pb::Takeovers {
            takeovers: records.values().cloned().collect(),
        }
    }
}

async fn watch_takeover(raft: Raft, group_id: u64, limit: Duration, records: Arc<Mutex<BTreeMap<u64, pb::Takeover>>>) {
    let armed_term = raft.metrics().borrow_watched().current_term;

    let Ok(elected) = raft.wait(Some(limit)).state(ServerState::Leader, "take over the group").await else {
        return;
    };
    let takeover = pb::Takeover {
        group_id,
        elected_ns: clock::monotonic_ns(),
        ready_ns: 0,
        terms: elected.current_term.saturating_sub(armed_term),
    };
    records.lock().expect("no thread panics while it holds the lock").insert(group_id, takeover);

    let ready = raft.wait(Some(limit)).metrics(is_ready_leader, "commit an entry of the new term").await;
    if ready.is_ok() {
        let ready_ns = clock::monotonic_ns();
        let mut records = records.lock().expect("no thread panics while it holds the lock");
        if let Some(takeover) = records.get_mut(&group_id) {
            takeover.ready_ns = ready_ns;
        }
    }
}
