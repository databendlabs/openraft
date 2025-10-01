use std::io::Cursor;
use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Vote;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::InstallSnapshotRequest;
use openraft::storage::Snapshot;
use openraft::storage::SnapshotMeta;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

///  API test: install_snapshot with vote lower than the target node.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn install_snapshot_lower_vote() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let mut log_index = 0;

    tracing::info!(log_index, "--- initializing cluster");
    log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let (n0, _, _) = router.remove_node(0).unwrap();
    let make_req = || InstallSnapshotRequest {
        vote: Vote::new_committed(2, 1),
        meta: SnapshotMeta {
            snapshot_id: "ss1".into(),
            last_log_id: Some(log_id(1, 0, 0)),
            last_membership: Default::default(),
        },
        offset: 0,
        data: vec![1, 2, 3],
        done: false,
    };

    tracing::info!(log_index, "--- force the vote on target node to be higher");
    {
        let _res = n0
            .append_entries(AppendEntriesRequest {
                vote: Vote::new_committed(2, 1),
                prev_log_id: None,
                entries: vec![],
                leader_commit: None,
            })
            .await;
        let vote = n0.with_raft_state(|st| *st.vote_ref()).await?;
        assert_eq!(Vote::new_committed(2, 1), vote);
    }

    tracing::info!(log_index, "--- install_snapshot with lower vote will be rejected");
    {
        let mut req = make_req();
        req.vote = Vote::new_committed(1, 1);

        let got = n0.install_snapshot(req).await?;
        assert_eq!(Vote::new_committed(2, 1), got.vote);

        let snapshot_meta = n0.with_raft_state(|st| st.snapshot_meta.clone()).await?;
        assert_eq!(SnapshotMeta::default(), snapshot_meta, "no snapshot is installed");
    }

    tracing::info!(log_index, "--- install_full_snapshot with lower vote will be rejected");
    {
        let mut req = make_req();
        req.vote = Vote::new_committed(1, 1);

        let got = n0
            .install_full_snapshot(Vote::new_committed(1, 1), Snapshot {
                meta: Default::default(),
                snapshot: Cursor::new(vec![]),
            })
            .await?;
        assert_eq!(Vote::new_committed(2, 1), got.vote);

        let snapshot_meta = n0.with_raft_state(|st| st.snapshot_meta.clone()).await?;
        assert_eq!(SnapshotMeta::default(), snapshot_meta, "no snapshot is installed");
    }
    Ok(())
}
