use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use tokio::task::AbortHandle;

use crate::Raft;
use crate::clock;
use crate::protobuf as pb;

/// Records how the members of this node rejoin their groups once the node returns from a fault:
/// when each member stops leading a group it led, and when each first accepts a leader of its
/// group or leads the group in a new term.
///
/// The server reports every AppendEntries request that a member accepts to [`Self::accepted()`],
/// which costs one atomic load until the rejoin is armed.
#[derive(Clone)]
pub(crate) struct Rejoins {
    inner: Arc<Inner>,
}

struct Inner {
    /// The `CLOCK_MONOTONIC` time, in nanoseconds, before which nothing is recorded; `u64::MAX`
    /// while the rejoin is not armed.
    after_ns: AtomicU64,

    /// When each member first accepted an AppendEntries request at or after `after_ns`, by group
    /// id; 0 for not yet. Set when the rejoin is armed.
    accepted_ns: OnceLock<Vec<AtomicU64>>,

    /// What the watchers recorded of each group, by group id: its stepdown, and when the member
    /// led it in a new term.
    records: Mutex<Vec<pb::Rejoin>>,

    /// The watchers that [`Rejoins::arm()`] started, which [`Rejoins::disarm()`] ends.
    watchers: Mutex<Vec<AbortHandle>>,
}

impl Default for Rejoins {
    fn default() -> Self {
        let inner = Inner {
            after_ns: AtomicU64::new(u64::MAX),
            accepted_ns: OnceLock::new(),
            records: Mutex::new(Vec::new()),
            watchers: Mutex::new(Vec::new()),
        };
        Self { inner: Arc::new(inner) }
    }
}

impl Rejoins {
    /// Record how the members of `groups` rejoin them from `after_ns` on, until
    /// [`Self::disarm()`].
    pub(crate) fn arm(&self, groups: &[Raft], after_ns: u64) -> anyhow::Result<()> {
        let accepted_ns = groups.iter().map(|_| AtomicU64::new(0)).collect();
        if self.inner.accepted_ns.set(accepted_ns).is_err() {
            anyhow::bail!("the rejoin is armed only once");
        }

        let mut records = self.inner.records.lock().expect("no thread panics while it holds the lock");
        let mut watchers = self.inner.watchers.lock().expect("no thread panics while it holds the lock");
        for (index, raft) in groups.iter().enumerate() {
            let (led, armed_term) = {
                let metrics = raft.metrics();
                let metrics = metrics.borrow_watched();
                (metrics.state == ServerState::Leader, metrics.current_term)
            };
            records.push(pb::Rejoin {
                group_id: index as u64,
                led,
                stepdown_ns: 0,
                rejoined_ns: 0,
            });

            let watcher = tokio::spawn(watch_rejoin(
                raft.clone(),
                index,
                led,
                armed_term,
                after_ns,
                self.inner.clone(),
            ));
            watchers.push(watcher.abort_handle());
        }

        self.inner.after_ns.store(after_ns, Ordering::Release);
        Ok(())
    }

    /// Record that the member of `group_id` replied to an AppendEntries request without rejecting
    /// the sender's vote, if the rejoin is armed and the member has not done so since `after_ns`.
    pub(crate) fn accepted(&self, group_id: u64) {
        let after_ns = self.inner.after_ns.load(Ordering::Acquire);
        if after_ns == u64::MAX {
            return;
        }
        let accepted_ns = self.inner.accepted_ns.get();
        let Some(accepted_ns) = accepted_ns.and_then(|accepted_ns| accepted_ns.get(group_id as usize)) else {
            return;
        };
        if accepted_ns.load(Ordering::Relaxed) != 0 {
            return;
        }

        let now = clock::monotonic_ns();
        if now >= after_ns {
            // A reply that another thread recorded first keeps its earlier time.
            accepted_ns.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed).ok();
        }
    }

    /// Return what was recorded of each group so far.
    pub(crate) fn recorded(&self) -> pb::Rejoins {
        let records = self.inner.records.lock().expect("no thread panics while it holds the lock");
        let accepted_ns = self.inner.accepted_ns.get();

        let rejoins = records.iter().map(|record| {
            let accepted_ns = accepted_ns.map_or(0, |accepted_ns| {
                accepted_ns[record.group_id as usize].load(Ordering::Relaxed)
            });
            pb::Rejoin {
                rejoined_ns: earliest(record.rejoined_ns, accepted_ns),
                ..*record
            }
        });
        pb::Rejoins {
            rejoins: rejoins.collect(),
        }
    }

    /// Stop recording, and end the watchers.
    pub(crate) fn disarm(&self) {
        self.inner.after_ns.store(u64::MAX, Ordering::Release);

        let mut watchers = self.inner.watchers.lock().expect("no thread panics while it holds the lock");
        for watcher in watchers.drain(..) {
            watcher.abort();
        }
    }
}

/// Watch the member `raft` of group `index` from `after_ns` on: record when it stops leading the
/// group, if it `led` it, and when it leads the group in a term above `armed_term`.
async fn watch_rejoin(raft: Raft, index: usize, led: bool, armed_term: u64, after_ns: u64, inner: Arc<Inner>) {
    let mut metrics = raft.metrics();
    let mut leading = led;

    loop {
        let (state, term) = {
            let metrics = metrics.borrow_watched();
            (metrics.state, metrics.current_term)
        };

        let now = clock::monotonic_ns();
        if now >= after_ns {
            let leads_new_term = state == ServerState::Leader && term > armed_term;

            let mut records = inner.records.lock().expect("no thread panics while it holds the lock");
            let record = &mut records[index];
            // A member that leads in a new term stopped leading in between, which the metrics, a
            // watch channel, may skip.
            if leading && (state != ServerState::Leader || leads_new_term) {
                record.stepdown_ns = now;
                leading = false;
            }
            if leads_new_term {
                record.rejoined_ns = now;
                return;
            }
        }

        if metrics.changed().await.is_err() {
            return;
        }
    }
}

/// Return the earlier of two times, where 0 stands for none.
fn earliest(a: u64, b: u64) -> u64 {
    match (a, b) {
        (0, b) => b,
        (a, 0) => a,
        (a, b) => a.min(b),
    }
}

#[cfg(test)]
mod tests {
    use super::Rejoins;
    use super::earliest;

    #[test]
    fn test_earliest_ignores_a_time_not_recorded() {
        assert_eq!(0, earliest(0, 0));
        assert_eq!(5, earliest(0, 5));
        assert_eq!(5, earliest(5, 0));
        assert_eq!(3, earliest(5, 3));
    }

    #[test]
    fn test_nothing_is_accepted_until_the_rejoin_is_armed() {
        let rejoins = Rejoins::default();

        rejoins.accepted(0);

        assert!(rejoins.recorded().rejoins.is_empty());
    }
}
