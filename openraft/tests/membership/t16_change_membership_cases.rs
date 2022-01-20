use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::NodeId;
use openraft::State;
use tracing_futures::Instrument;

use crate::fixtures::RaftRouter;

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn change_membership_cases() -> anyhow::Result<()> {
    let (_log_guard, ut_span) = init_ut!();

    async {
        change_from_to(btreeset! {0}, btreeset! {1}).await?;
        change_from_to(btreeset! {0}, btreeset! {1,2}).await?;
        change_from_to(btreeset! {0}, btreeset! {1,2, 3}).await?;
        change_from_to(btreeset! {0, 1}, btreeset! {1, 2}).await?;
        change_from_to(btreeset! {0, 1}, btreeset! {1}).await?;
        change_from_to(btreeset! {0, 1}, btreeset! {2}).await?;
        change_from_to(btreeset! {0, 1}, btreeset! {3}).await?;
        change_from_to(btreeset! {0, 1, 2}, btreeset! {4}).await?;
        change_from_to(btreeset! {0, 1, 2}, btreeset! {4,5,6}).await?;
        change_from_to(btreeset! {0, 1, 2, 3, 4}, btreeset! {0, 1, 2, 3}).await?;

        Ok::<(), anyhow::Error>(())
    }
    .instrument(ut_span)
    .await?;

    Ok(())
}

#[tracing::instrument(level = "debug")]
async fn change_from_to(old: BTreeSet<NodeId>, new: BTreeSet<NodeId>) -> anyhow::Result<()> {
    let mes = format!("from {:?} to {:?}", old, new);

    let only_in_old = old.difference(&new);
    let only_in_new = new.difference(&old);

    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(old.clone(), btreeset! {}).await?;

    tracing::info!("--- write 100 logs");
    {
        router.client_request_many(0, "client", 100).await;
        log_index += 100;

        router.wait_for_log(&old, Some(log_index), timeout(), &format!("write 100 logs, {}", mes)).await?;
    }

    // let mtx = router.wait(&0, timeout()).await?.log(Some(0), "get metrics").await?;
    // let term_0 = mtx.current_term;
    let orig_leader = router.leader().await.expect("expected the cluster to have a leader");

    tracing::info!("--- change to {:?}", new);
    {
        for id in only_in_new {
            router.new_raft_node(*id).await;
        }

        router.change_membership(0, new.clone()).await?;
        log_index += 2; // two member-change logs

        tracing::info!("--- wait for old leader or new leader");
        {
            for id in new.iter() {
                router
                    .wait(id, Some(Duration::from_millis(5_000)))
                    .await?
                    .metrics(
                        |x| x.current_leader.is_some() && new.contains(&x.current_leader.unwrap()),
                        format!("node {} in new cluster has leader in new cluster, {}", id, mes),
                    )
                    .await?;
            }
        }

        let new_leader = router.leader().await.expect("expected the cluster to have a leader");
        for id in new.iter() {
            // new leader may already elected and committed a blank log.
            router
                .wait(id, timeout())
                .await?
                .log_at_least(Some(log_index), format!("new cluster, {}", mes))
                .await?;

            if new_leader != orig_leader {
                router
                    .wait(&id, timeout())
                    .await?
                    .metrics(
                        |x| x.current_term >= 2,
                        "new cluster has term >= 2 because of new election",
                    )
                    .await?;
            }
        }

        for id in only_in_old.clone() {
            router
                .wait(id, timeout())
                .await?
                .state(State::Learner, format!("node {} only in old, {}", id, mes))
                .await?;
        }
    }

    tracing::info!("--- write another 100 logs");
    {
        // get new leader

        let m = router
            .wait(new.iter().next().unwrap(), timeout())
            .await?
            .metrics(|x| x.current_leader.is_some(), format!("wait for new leader, {}", mes))
            .await?;

        let leader = m.current_leader.unwrap();

        router.client_request_many(leader, "client", 100).await;
        log_index += 100;
    }

    for id in new.iter() {
        router
            .wait(id, timeout())
            .await?
            // new leader may commit a blonk log
            .log_at_least(Some(log_index), format!("new cluster recv logs 100~200, {}", mes))
            .await?;
    }

    tracing::info!("--- log will not be sync to removed node");
    {
        for id in only_in_old {
            let res = router
                .wait(id, timeout())
                .await?
                .log(
                    Some(log_index),
                    format!("node {} in old cluster wont recv new logs, {}", id, mes),
                )
                .await;
            assert!(res.is_err());
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
