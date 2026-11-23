use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::async_runtime::watch::RecvError;
use openraft::errors::Fatal;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;
#[allow(unused_imports)]
use pretty_assertions::assert_eq;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::ut_harness;
use crate::t10_metrics_recorder::TestRecorder;

/// A node that stops reports `Shutdown` through its server metrics and its metrics recorder, not
/// only through its full metrics, whether a shutdown or a storage error stopped it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shutdown_reaches_every_metrics_surface() -> Result<()> {
    for apply_error in [false, true] {
        let config = Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;

        tracing::info!(apply_error, "--- bring up one node with a metrics recorder");
        let (raft, recorder) = {
            let log_index = router.new_cluster([0], []).await?;
            let raft = router.get_raft_handle(&0)?;
            let recorder = Arc::new(TestRecorder::default());
            raft.set_metrics_recorder(Some(recorder.clone())).await?;

            // The recorder gets values from the next metrics report, such as that of a write.
            router.client_request(0, "foo", 1).await?;
            router.wait(&0).applied_index(Some(log_index + 1), "write applied").await?;
            (raft, recorder)
        };

        let all = raft.metrics();
        let mut server = raft.server_metrics();
        let before = server.borrow_and_update().clone();
        assert_eq!(ServerState::Leader, before.state);
        assert_eq!(3, recorder.server_state.load(Ordering::Relaxed), "3 is Leader");

        tracing::info!(apply_error, "--- stop the node");
        let fatal = {
            let fatal = if apply_error {
                let (_log_store, state_machine) = router.get_storage_handle(&0)?;
                state_machine.fail_next_apply(io::Error::other("apply failure"));
                let error = router.client_request(0, "foo", 2).await.unwrap_err();
                error.into_fatal().expect("a failed apply stops the node")
            } else {
                Fatal::Stopped
            };
            // Joining the core task waits for its terminal metrics.
            raft.shutdown().await?;
            fatal
        };

        tracing::info!(apply_error, "--- every metrics surface reports the stopped node");
        {
            let mut want = before.clone();
            want.state = ServerState::Shutdown;
            TypeConfig::timeout(WAIT_TIMEOUT, server.changed()).await??;
            assert_eq!(want, *server.borrow_watched(), "only the server state changes");
            let closed = TypeConfig::timeout(WAIT_TIMEOUT, server.changed()).await?;
            assert_eq!(Err(RecvError(())), closed, "then the channel closes");

            let full = all.borrow_watched().clone();
            assert_eq!(ServerState::Shutdown, full.state);
            assert_eq!(Err(fatal), full.running_state);

            assert_eq!(4, recorder.server_state.load(Ordering::Relaxed), "4 is Shutdown");
        }
    }

    Ok(())
}
