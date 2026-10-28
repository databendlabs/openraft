use std::io::Cursor;
use std::pin::pin;
use std::time::Duration;

use anyhow::Result;
use futures::future::Either;
use openraft::Config;
use openraft::RPCTypes;
use openraft::RaftNetworkFactory;
use openraft::RaftNetworkV2;
use openraft::Vote;
use openraft::alias::SnapshotOf;
use openraft::errors::Infallible;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::network::RPCOption;
use openraft::type_config::OneshotSender;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::OperationGate;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Direct snapshot transfer returns `Closed` when cancellation wins over a paused pre-hook.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_cancellation_stops_at_hook_direct() -> Result<()> {
    cancel_paused_snapshot(false).await
}

/// Chunked snapshot transfer returns `Closed` when cancellation wins over a paused pre-hook.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_cancellation_stops_at_hook_chunked() -> Result<()> {
    cancel_paused_snapshot(true).await
}

async fn cancel_paused_snapshot(chunked: bool) -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let builder = RaftRouter::builder(config)?;
    let mut router = build_router(builder, chunked);

    let gate = OperationGate::default();
    let hook_gate = gate.clone();
    router
        .set_rpc_pre_hook(RPCTypes::InstallSnapshot, move |_router, _request, _from, _to| {
            let gate = hook_gate.clone();
            Box::pin(async move {
                gate.block().await;
                Ok::<(), RPCError<TypeConfig, Infallible>>(())
            })
        })
        .await;

    let mut network = router.new_client(1, &()).await;
    let closed = ReplicationClosed::new("snapshot transfer cancelled");
    let signal = closed.clone();
    let (sender, receiver) = TypeConfig::oneshot();
    let cancel = async move {
        let received = receiver.await;
        received.expect("cancellation is signaled while the hook is paused")
    };
    let vote = Vote::new_committed(1, 0);
    let snapshot = SnapshotOf::<TypeConfig, Cursor<Vec<u8>>> {
        meta: Default::default(),
        snapshot: Cursor::new(vec![]),
    };
    let deadline = Duration::from_millis(50);
    let option = RPCOption::new(deadline);
    let transfer = async move {
        let call = network.full_snapshot(vote, snapshot, cancel, option);
        let call = pin!(call);
        let entered = gate.entered();
        let entered = pin!(entered);
        let raced = futures::future::select(entered, call).await;
        match raced {
            Either::Left((_entered, call)) => {
                let sent = sender.send(signal);
                sent.expect("the snapshot call is still waiting");
                call.await
            }
            Either::Right((result, _entered)) => result,
        }
    };
    let observed = TypeConfig::timeout(deadline, transfer).await;
    assert_eq!(Ok(Err(StreamingError::Closed(closed))), observed, "chunked={chunked}");
    Ok(())
}

fn build_router(builder: crate::fixtures::Builder, chunked: bool) -> RaftRouter {
    if chunked {
        let builder = builder.chunked_snapshots();
        return builder.build();
    }
    builder.build()
}
