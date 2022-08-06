use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use memstore::ClientRequest;
use openraft::raft::VoteRequest;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::ServerState;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// A leader reverts to follower if a higher vote is seen when append-entries.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_sees_higher_vote() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let _log_index = router.new_nodes_from_single(btreeset! {0,1}, btreeset! {}).await?;

    tracing::info!("--- upgrade vote on node-1");
    {
        router
            .connect(1, &())
            .await?
            .send_vote(VoteRequest {
                vote: Vote::new(10, 1),
                last_log_id: Some(LogId::new(LeaderId::new(10, 1), 5)),
            })
            .await?;
    }

    tracing::info!("--- a write operation will see a higher vote, then the leader revert to follower");
    {
        router.wait(&0, timeout()).state(ServerState::Leader, "node-0 is leader").await?;

        let n0 = router.get_raft_handle(&0)?;
        let res = n0
            .client_write(ClientRequest {
                client: "0".to_string(),
                serial: 1,
                status: "2".to_string(),
            })
            .await;

        tracing::debug!("--- client_write res: {:?}", res);

        router
            .wait(&0, timeout())
            .state(ServerState::Follower, "node-0 becomes follower due to a higher vote")
            .await?;

        router.external_request(0, |st, _, _| {
            assert_eq!(Vote::new(10, 1), st.vote, "higher vote is stored");
        });
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
