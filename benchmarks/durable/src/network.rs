//! Fault gates around existing unary HTTP adapters, including complete snapshot transfers.

use std::future::Future;
use std::sync::Arc;

use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::alias::SnapshotOf;
use openraft::alias::VoteOf;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::errors::Unreachable;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderRequest;
use openraft::raft::TransferLeaderResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;

use crate::faults::Faults;

pub(crate) struct Factory<F> {
    inner: F,
    faults: Arc<Faults>,
}

pub(crate) struct Network<N> {
    inner: N,
    target: String,
    faults: Arc<Faults>,
}

impl<F> Factory<F> {
    pub(crate) fn new(inner: F, faults: Arc<Faults>) -> Self {
        Self { inner, faults }
    }
}

impl<C, F> RaftNetworkFactory<C> for Factory<F>
where
    C: RaftTypeConfig,
    F: RaftNetworkFactory<C>,
    F::Network: RaftNetworkV2<C>,
{
    type Network = Network<F::Network>;

    async fn new_client(&mut self, target: C::NodeId, node: &C::Node) -> Self::Network {
        let id = target.to_string();
        let inner = self.inner.new_client(target, node).await;
        Network {
            inner,
            target: id,
            faults: self.faults.clone(),
        }
    }
}

impl<C, N> RaftNetworkV2<C> for Network<N>
where
    C: RaftTypeConfig,
    N: RaftNetworkV2<C>,
{
    type SnapshotData = N::SnapshotData;

    async fn append_entries(
        &mut self,
        request: AppendEntriesRequest<C>,
        option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        let faults = self.faults.clone();
        let response = faults.run(&self.target, self.inner.append_entries(request, option)).await;
        response.map_err(unavailable::<C>)?
    }

    async fn vote(&mut self, request: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let faults = self.faults.clone();
        let response = faults.run(&self.target, self.inner.vote(request, option)).await;
        response.map_err(unavailable::<C>)?
    }

    async fn pre_vote(&mut self, request: VoteRequest<C>, option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        let faults = self.faults.clone();
        let response = faults.run(&self.target, self.inner.pre_vote(request, option)).await;
        response.map_err(unavailable::<C>)?
    }

    async fn transfer_leader(
        &mut self,
        request: TransferLeaderRequest<C>,
        option: RPCOption,
    ) -> Result<TransferLeaderResponse<C>, RPCError<C>> {
        let faults = self.faults.clone();
        let response = faults.run(&self.target, self.inner.transfer_leader(request, option)).await;
        response.map_err(unavailable::<C>)?
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<C>,
        snapshot: SnapshotOf<C, Self::SnapshotData>,
        cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        let faults = self.faults.clone();
        let pending = std::future::pending::<ReplicationClosed>();
        let transfer = self.inner.full_snapshot(vote, snapshot, pending, option);
        tokio::pin!(cancel);
        tokio::select! {
            closed = &mut cancel => Err(StreamingError::Closed(closed)),
            response = faults.run(&self.target, transfer) => response
                .map_err(|error| StreamingError::Unreachable(Unreachable::new(&error)))?,
        }
    }
}

fn unavailable<C>(error: std::io::Error) -> RPCError<C>
where C: RaftTypeConfig {
    RPCError::Unreachable(Unreachable::new(&error))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::io::Cursor;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use raft_kv_log_wal_sm_mem::TypeConfig;

    use super::*;

    struct PendingSnapshot {
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        dropped: Arc<AtomicBool>,
    }

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    impl RaftNetworkV2<TypeConfig> for PendingSnapshot {
        type SnapshotData = Cursor<Vec<u8>>;

        async fn append_entries(
            &mut self,
            _: AppendEntriesRequest<TypeConfig>,
            _: RPCOption,
        ) -> Result<AppendResult<TypeConfig>, RPCError<TypeConfig>> {
            Err(unavailable(io::Error::other("snapshot-only fixture")))
        }

        async fn vote(
            &mut self,
            _: VoteRequest<TypeConfig>,
            _: RPCOption,
        ) -> Result<VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
            Err(unavailable(io::Error::other("snapshot-only fixture")))
        }

        async fn full_snapshot(
            &mut self,
            _: VoteOf<TypeConfig>,
            _: SnapshotOf<TypeConfig, Self::SnapshotData>,
            _: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
            _: RPCOption,
        ) -> Result<SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
            let _released = DropFlag(self.dropped.clone());
            self.entered.take().expect("one transfer").send(()).unwrap();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn cancellation_drops_the_complete_inner_snapshot_future() -> io::Result<()> {
        let dropped = Arc::new(AtomicBool::new(false));
        let (entered, started) = tokio::sync::oneshot::channel();
        let inner = PendingSnapshot {
            entered: Some(entered),
            dropped: dropped.clone(),
        };
        let mut network = Network {
            inner,
            target: "2".into(),
            faults: Faults::new(),
        };
        let snapshot = SnapshotOf::<TypeConfig, Cursor<Vec<u8>>> {
            meta: openraft::alias::SnapshotMetaOf::<TypeConfig> {
                last_log_id: None,
                last_membership: Default::default(),
            },
            snapshot: Cursor::new(vec![]),
        };
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let vote = openraft::impls::Vote::new_committed(1, 1);
            let cancel = async move { cancelled.await.expect("cancel signal") };
            network.full_snapshot(vote, snapshot, cancel, RPCOption::new(Duration::from_secs(60))).await
        });
        started.await.map_err(io::Error::other)?;
        let reason = ReplicationClosed::new("cancel stalled response body");
        cancel.send(reason.clone()).unwrap();
        let result = handle.await.map_err(io::Error::other)?;
        let error = result.unwrap_err();
        match error {
            StreamingError::Closed(actual) => assert_eq!(reason, actual),
            error => panic!("unexpected snapshot error: {error}"),
        }
        let released = dropped.load(Ordering::SeqCst);
        assert!(released);
        Ok(())
    }
}
