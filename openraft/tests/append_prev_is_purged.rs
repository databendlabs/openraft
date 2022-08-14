use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::AppendEntriesRequest;
use openraft::Config;
use openraft::DefensiveCheck;
use openraft::LogId;
use openraft::Membership;
use openraft::Raft;
use openraft::RaftStorage;

use crate::fixtures::blank;
use crate::fixtures::RaftRouter;

#[macro_use]
mod fixtures;

/// When handling append-entries, if the local log at `prev_log_id.index` is purged, a follower should not believe it is
/// a **conflict** and should not delete all logs. Which will get committed log lost.
///
/// Fake a raft node with one log (1,3) and set last-applied to (1,2).
/// Then an append-entries with `prev_log_id=(1,2)` should not be considered as **conflict**.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn append_prev_is_purged() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(
        Config {
            max_applied_log_to_keep: 2,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    tracing::info!("--- fake store: logs: (1,3), last_applied == last_purged == (1,2)");
    let sto0 = {
        let sto0 = router.new_store(0).await;

        // With defensive==true, it will panic.
        sto0.set_defensive(false);

        let entries = [
            &Entry {
                log_id: LogId { term: 0, index: 0 },
                payload: EntryPayload::Membership(Membership::new_single(btreeset! {0,1})),
            },
            &blank(1, 1),
            &blank(1, 2),
            &blank(1, 3),
        ];

        sto0.append_to_log(&entries).await?;
        sto0.apply_to_state_machine(&entries[0..3]).await?;
        sto0.delete_logs_from(0..3).await?;

        let logs = sto0.try_get_log_entries(..).await?;
        tracing::debug!("logs left after purge: {:?}", logs);
        assert_eq!(LogId::new(1, 3), logs[0].log_id);

        sto0
    };

    tracing::info!("--- new node with faked sto");
    let node0 = {
        let config0 = Arc::new(
            Config {
                max_applied_log_to_keep: 1,
                ..Default::default()
            }
            .validate()?,
        );
        let node0 = Raft::new(0, config0.clone(), router.clone(), sto0.clone());
        router.add_raft_node(0, node0.clone(), sto0.clone()).await;
        node0
    };

    tracing::info!("--- append-entries with prev_log_id=(1,2), should not erase any logs");
    {
        node0
            .append_entries(AppendEntriesRequest {
                term: 1,
                leader_id: 1,
                prev_log_id: LogId::new(1, 2),
                entries: vec![],
                leader_commit: LogId::new(0, 0),
            })
            .await?;

        let logs = sto0.try_get_log_entries(..).await?;
        tracing::debug!("logs left after append: {:?}", logs);
        assert_eq!(LogId::new(1, 3), logs[0].log_id);
    }

    Ok(())
}
