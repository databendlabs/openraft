use anyhow::Context;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use raft_kv_rocksdb::TypeConfig;

use crate::node::Node;

#[test]
fn test_snapshot_followed_by_logs() -> anyhow::Result<()> {
    const PORT_BASE: u16 = 19000;
    TypeConfig::run(async {
        let leader = Node::new(1, PORT_BASE).await?;
        let learner = Node::new(2, PORT_BASE).await?;
        leader.initialize().await?;

        tracing::info!("Freeze and purge a snapshot before creating the log tail");
        let snapshot = {
            leader.write("shared", "before").await?;
            let snapshot = leader.build_snapshot().await?;
            leader.purge(&snapshot).await?;
            snapshot
        };

        tracing::info!(snapshot = ?snapshot.meta.last_log_id, "Write a tail that overwrites snapshot data");
        let (overwritten, added) = {
            let overwritten = leader.write("shared", "after").await?;
            let added = leader.write("tail", "new").await?;
            (overwritten, added)
        };

        tracing::info!("Join a learner that needs both the frozen checkpoint and later logs");
        {
            leader.add_learner(&learner).await?;
            let metrics = learner.raft().metrics();
            let installed = metrics.borrow_watched().snapshot;
            assert_eq!(snapshot.meta.last_log_id, installed);
            let shared = learner.read("shared").await?;
            assert_eq!(overwritten.data, shared);
            let tail = learner.read("tail").await?;
            assert_eq!(added.data, tail);
            let expected_state = leader.applied_state().await?;
            let actual_state = learner.applied_state().await?;
            assert_eq!(expected_state, actual_state);
        }

        tracing::info!("Continue writing through the writable database created from the checkpoint");
        {
            let value = overwritten.data.value.context("overwritten value exists")?;
            let request = types_kv::Request::compare_and_set("shared", value.version, "cas");
            let response = leader.raft().client_write(request).await?;
            let index = response.log_id.index();
            learner.wait_applied(index).await?;
            let expected = types_kv::Response::new("cas", index);
            assert_eq!(expected, response.data);
            let actual = learner.read("shared").await?;
            assert_eq!(expected, actual);
        }

        leader.cleanup().await?;
        learner.cleanup().await?;
        Ok(())
    })
}

#[test]
fn test_restart_after_snapshot_installation() -> anyhow::Result<()> {
    const PORT_BASE: u16 = 29000;
    TypeConfig::run(async {
        let leader = Node::new(1, PORT_BASE).await?;
        let mut learner = Node::new(2, PORT_BASE).await?;
        leader.initialize().await?;

        tracing::info!("Install a checkpoint on a learner whose source logs have been purged");
        let expected = {
            let written = leader.write("key", "snapshot value").await?;
            let snapshot = leader.build_snapshot().await?;
            leader.purge(&snapshot).await?;
            leader.add_learner(&learner).await?;
            written.data
        };
        let expected_state = learner.applied_state().await?;
        let expected_current = learner.current()?;

        tracing::info!(dir = %learner.dir().display(), "Reopen the learner's databases on a new runtime");
        {
            learner.restart().await?;
            let actual = learner.read("key").await?;
            assert_eq!(expected, actual);
            let actual_state = learner.applied_state().await?;
            assert_eq!(expected_state, actual_state);
            let actual_current = learner.current()?;
            assert_eq!(expected_current, actual_current);
        }

        tracing::info!("Reconnect replication and apply a new write after restart");
        {
            let response = leader.write("key", "after restart").await?;
            let index = response.log_id.index();
            learner.wait_applied(index).await?;
            let actual = learner.read("key").await?;
            assert_eq!(response.data, actual);
            let expected_state = leader.applied_state().await?;
            let actual_state = learner.applied_state().await?;
            assert_eq!(expected_state, actual_state);
        }

        leader.cleanup().await?;
        learner.cleanup().await?;
        Ok(())
    })
}

#[test]
fn test_restarted_leader_rebuilds_snapshot_for_learner() -> anyhow::Result<()> {
    const PORT_BASE: u16 = 9000;
    TypeConfig::run(async {
        let mut leader = Node::new(1, PORT_BASE).await?;
        let learner = Node::new(2, PORT_BASE).await?;
        leader.initialize().await?;

        tracing::info!("Persist a checkpoint and purge every log it covers before restarting");
        let (expected, snapshot) = {
            let written = leader.write("key", "survives purge").await?;
            let snapshot = leader.build_snapshot().await?;
            leader.purge(&snapshot).await?;
            (written.data, snapshot)
        };
        let expected_current = leader.current()?;
        drop(snapshot.snapshot);

        tracing::info!(dir = %leader.dir().display(), "Recover the active database and rebuild its snapshot");
        {
            leader.restart().await?;
            let actual = leader.read("key").await?;
            assert_eq!(expected, actual);
            let actual_current = leader.current()?;
            assert_eq!(expected_current, actual_current);
            let rebuilt = leader.raft().get_snapshot().await?;
            let rebuilt = rebuilt.context("startup rebuilds the purged snapshot")?;
            assert_eq!(snapshot.meta, rebuilt.meta);
        }

        tracing::info!("Catch up a new learner using the snapshot rebuilt after restart");
        {
            leader.add_learner(&learner).await?;
            let metrics = learner.raft().metrics();
            let installed = metrics.borrow_watched().snapshot;
            assert_eq!(snapshot.meta.last_log_id, installed);
            let actual = learner.read("key").await?;
            assert_eq!(expected, actual);
            let expected_state = leader.applied_state().await?;
            let actual_state = learner.applied_state().await?;
            assert_eq!(expected_state, actual_state);
        }

        leader.cleanup().await?;
        learner.cleanup().await?;
        Ok(())
    })
}
