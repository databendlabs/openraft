use std::io;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use maplit::btreeset;
use openraft::Config;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::StoredMembership;
use openraft::alias::LogIdOf;
use openraft::error::Fatal;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use openraft::storage::SnapshotMeta;
use openraft_memstore::TypeConfig;

use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Access [`RaftStateMachine`] via
/// [`Raft::with_state_machine()`](openraft::Raft::with_state_machine)
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn with_state_machine() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!("--- get last applied from SM");
    {
        let applied = n0
            .with_state_machine(|sm: &mut MemStateMachine| {
                Box::pin(async move {
                    let d = sm.get_state_machine().await;
                    d.last_applied_log
                })
            })
            .await?
            .unwrap();
        assert_eq!(applied, Some(log_id(1, 0, log_index)));
    }

    tracing::info!("--- shutting down node 0");
    n0.shutdown().await?;

    let res = n0.with_state_machine(|_sm: &mut MemStateMachine| Box::pin(async move {})).await;
    assert_eq!(Err(Fatal::Stopped), res);

    Ok(())
}

/// Call [`Raft::with_state_machine()`](openraft::Raft::with_state_machine) with wrong type
/// [`RaftStateMachine`]
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn with_state_machine_wrong_sm_type() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!("--- use wrong type SM");
    {
        type TC = TypeConfig;
        type Err = io::Error;
        struct FooSM;
        impl RaftSnapshotBuilder<TC> for FooSM {
            async fn build_snapshot(&mut self) -> Result<Snapshot<TC>, Err> {
                todo!()
            }
        }
        impl RaftStateMachine<TC> for FooSM {
            type SnapshotBuilder = Self;

            async fn applied_state(&mut self) -> Result<(Option<LogIdOf<TypeConfig>>, StoredMembership<TC>), Err> {
                todo!()
            }

            async fn apply<Strm>(&mut self, _entries: Strm) -> Result<(), Err>
            where Strm: Stream<Item = Result<EntryResponder<TC>, Err>> + Unpin + OptionalSend {
                todo!()
            }

            async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
                todo!()
            }

            async fn begin_receiving_snapshot(&mut self) -> Result<<TC as RaftTypeConfig>::SnapshotData, Err> {
                todo!()
            }

            async fn install_snapshot(
                &mut self,
                _meta: &SnapshotMeta<TC>,
                _snapshot: <TC as RaftTypeConfig>::SnapshotData,
            ) -> Result<(), Err> {
                todo!()
            }

            async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TC>>, Err> {
                todo!()
            }
        }

        let applied = n0.with_state_machine::<_, FooSM, _>(|_sm: &mut FooSM| Box::pin(async move {})).await?;
        assert!(applied.is_err());
    }

    Ok(())
}
