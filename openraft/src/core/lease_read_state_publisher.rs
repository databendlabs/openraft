use std::sync::Arc;

use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchSender;
use crate::raft::linearizable_read::lease_read_state::LeaseReadState;
use crate::type_config::alias::WatchSenderOf;

/// Owns the readable snapshot and clears it even when the core unwinds or is cancelled.
pub(crate) struct LeaseReadStatePublisher<C>
where C: RaftTypeConfig
{
    tx: WatchSenderOf<C, Option<Arc<LeaseReadState<C>>>>,
}

impl<C> LeaseReadStatePublisher<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(tx: WatchSenderOf<C, Option<Arc<LeaseReadState<C>>>>) -> Self {
        Self { tx }
    }

    /// Publishes changed snapshots, allocating an `Arc` only when their value differs.
    pub(crate) fn publish(&self, state: Option<LeaseReadState<C>>) -> bool {
        self.tx.send_if_modified(|published| {
            let current = published.as_deref();
            let next = state.as_ref();
            if current == next {
                return false;
            }
            let Some(state) = state else {
                *published = None;
                return true;
            };
            let next = Arc::new(state);
            *published = Some(next);
            true
        })
    }

    pub(crate) fn clear(&self) -> bool {
        self.publish(None)
    }
}

impl<C> Drop for LeaseReadStatePublisher<C>
where C: RaftTypeConfig
{
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod lease_read_state_publisher_test;
