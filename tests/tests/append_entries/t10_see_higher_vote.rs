use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::network::RPCOption;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use openraft::raft::VoteRequest;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::ServerState;
use openraft::Vote;
use openraft_memstore::ClientRequest;
use tokio::time::sleep;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// A leader reverts to follower if a higher vote is seen when append-entries.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_sees_higher_vote() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            election_timeout_min: 500,
            election_timeout_max: 501,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let _log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    tracing::info!("--- upgrade vote on node-1");
    {
        // Let leader lease expire
        sleep(Duration::from_millis(800)).await;

        let option = RPCOption::new(Duration::from_millis(1_000));

        let resp = router
            .new_client(1, &())
            .await
            .vote(
                VoteRequest {
                    vote: Vote::new(10, 1),
                    last_log_id: Some(LogId::new(CommittedLeaderId::new(10, 1), 5)),
                },
                option,
            )
            .await?;

        assert!(resp.vote_granted);
    }

    // Current state:
    // n0: vote=(1,0)
    // n1: vote=(10,1)
    tracing::info!("--- a write operation will see a higher vote, then the leader revert to follower");
    {
        router.wait(&0, timeout()).state(ServerState::Leader, "node-0 is leader").await?;

        let n0 = router.get_raft_handle(&0)?;
        tokio::spawn(async move {
            let res = n0
                .client_write(ClientRequest {
                    client: "0".to_string(),
                    serial: 1,
                    status: "2".to_string(),
                })
                .await;

            tracing::debug!("--- client_write res: {:?}", res);
        });

        tokio::time::sleep(Duration::from_millis(500)).await;

        router
            .wait(&0, timeout())
            .state(ServerState::Follower, "node-0 becomes follower due to a higher vote")
            .await?;

        router.external_request(0, |st| {
            assert_eq!(&Vote::new(10, 1), st.vote_ref(), "higher vote is stored");
        });
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
