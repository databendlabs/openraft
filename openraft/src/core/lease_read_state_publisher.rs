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
    /// Compare unchanged state without acquiring the watch channel's write lock.
    published: Option<Arc<LeaseReadState<C>>>,
}

impl<C> LeaseReadStatePublisher<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(tx: WatchSenderOf<C, Option<Arc<LeaseReadState<C>>>>) -> Self {
        let published = tx.borrow_watched().clone();
        Self { tx, published }
    }

    /// Allocates and takes the watch write lock only when the authorization state changes.
    pub(crate) fn publish(&mut self, state: Option<LeaseReadState<C>>) -> bool {
        let current = self.published.as_deref();
        if current == state.as_ref() {
            return false;
        }
        let next = match state {
            Some(state) => {
                let state = Arc::new(state);
                Some(state)
            }
            None => None,
        };
        self.published = next.clone();
        self.tx.send_if_modified(|published| {
            *published = next;
            true
        })
    }

    pub(crate) fn clear(&mut self) -> bool {
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
