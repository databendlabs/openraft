use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use tokio::sync::watch;

/// Whether this node is cut off from its peers, as with a blackholed network: while it is cut,
/// its router holds each Raft request to a peer without sending it, and its server holds each
/// Raft request from a peer without passing it on.
///
/// A held request gets no reply, so a sender with a timeout drops it. Openraft keeps a heartbeat
/// or a vote in flight instead, until it gets a reply. A request whose sender still waits at the
/// heal goes on, as data that TCP sends again once a blackhole ends.
#[derive(Clone, Default)]
pub(crate) struct Partition {
    inner: Arc<Inner>,
}

struct Inner {
    /// Read by every Raft request, so that a node that is not cut pays one atomic load.
    cut: AtomicBool,

    /// Wakes the held requests at the heal.
    cut_tx: watch::Sender<bool>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            cut: AtomicBool::new(false),
            cut_tx: watch::Sender::new(false),
        }
    }
}

impl Partition {
    pub(crate) fn cut(&self) {
        self.inner.cut.store(true, Ordering::Relaxed);
        self.inner.cut_tx.send_replace(true);
    }

    pub(crate) fn heal(&self) {
        self.inner.cut.store(false, Ordering::Relaxed);
        self.inner.cut_tx.send_replace(false);
    }

    /// Wait while the links are cut.
    pub(crate) async fn hold(&self) {
        if !self.inner.cut.load(Ordering::Relaxed) {
            return;
        }

        let mut cut = self.inner.cut_tx.subscribe();
        // `inner` keeps the sender, so the wait ends only at the heal.
        cut.wait_for(|cut| !*cut).await.ok();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Partition;

    #[tokio::test]
    async fn test_a_cut_holds_a_request_until_the_heal() {
        let partition = Partition::default();

        partition.cut();
        let held = tokio::spawn({
            let partition = partition.clone();
            async move { partition.hold().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!held.is_finished(), "the cut holds the request");

        partition.heal();
        tokio::time::timeout(Duration::from_secs(5), held)
            .await
            .expect("the heal releases the request")
            .expect("the held request does not panic");
    }
}
