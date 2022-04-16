use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use memstore::MemNodeId;
use openraft::ChangeMembers;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn change_membership_cases() -> anyhow::Result<()> {
    change_from_to(btreeset! {0}, ChangeMembers::Replace(btreeset! {1})).await?;
    change_from_to(btreeset! {0}, ChangeMembers::Replace(btreeset! {1,2})).await?;
    change_from_to(btreeset! {0}, ChangeMembers::Replace(btreeset! {1,2,3})).await?;
    change_from_to(btreeset! {0, 1}, ChangeMembers::Replace(btreeset! {1,2})).await?;
    change_from_to(btreeset! {0, 1}, ChangeMembers::Replace(btreeset! {1})).await?;
    change_from_to(btreeset! {0, 1}, ChangeMembers::Replace(btreeset! {2})).await?;
    change_from_to(btreeset! {0, 1}, ChangeMembers::Replace(btreeset! {3})).await?;
    change_from_to(btreeset! {0, 1, 2}, ChangeMembers::Replace(btreeset! {4})).await?;
    change_from_to(btreeset! {0, 1, 2}, ChangeMembers::Replace(btreeset! {4,5,6})).await?;
    change_from_to(btreeset! {0, 1, 2, 3, 4}, ChangeMembers::Replace(btreeset! {0,1,2,3})).await?;

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn add_members_cases() -> anyhow::Result<()> {
    change_from_to(btreeset! {0}, ChangeMembers::Add(btreeset! {0,1})).await?;
    change_from_to(btreeset! {0}, ChangeMembers::Add(btreeset! {1,2})).await?;
    change_from_to(btreeset! {0,1}, ChangeMembers::Add(btreeset! {})).await?;

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn remove_members_cases() -> anyhow::Result<()> {
    change_from_to(btreeset! {0,1,2}, ChangeMembers::Remove(btreeset! {0,1})).await?;
    change_from_to(btreeset! {0,1,2}, ChangeMembers::Remove(btreeset! {3})).await?;
    change_from_to(btreeset! {0,1,2}, ChangeMembers::Remove(btreeset! {})).await?;
    change_from_to(btreeset! {0,1,2}, ChangeMembers::Remove(btreeset! {1,3})).await?;

    Ok(())
}

#[tracing::instrument(level = "debug")]
async fn change_from_to(old: BTreeSet<MemNodeId>, change_members: ChangeMembers<MemNodeId>) -> anyhow::Result<()> {
    let new = change_members.apply_to(&old);

    let mes = format!("from {:?} to {:?}", old, new);

    let only_in_old = old.difference(&new);
    let only_in_new = new.difference(&old);

    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(old.clone(), btreeset! {}).await?;

    tracing::info!("--- write 100 logs");
    {
        router.client_request_many(0, "client", 100).await;
        log_index += 100;

        router.wait_for_log(&old, Some(log_index), timeout(), &format!("write 100 logs, {}", mes)).await?;
    }

    let orig_leader = router.leader().expect("expected the cluster to have a leader");

    tracing::info!("--- change to {:?}", new);
    {
        for id in only_in_new {
            router.new_raft_node(*id).await;
            router.add_learner(0, *id).await?;
            log_index += 1;
            router.wait_for_log(&old, Some(log_index), timeout(), &format!("add learner, {}", mes)).await?;
        }

        let node = router.get_raft_handle(&0)?;
        node.change_membership(new.clone(), true, false).await?;
        log_index += 1;
        if new != old {
            log_index += 1; // two member-change logs
        }

        tracing::info!("--- wait for old leader or new leader");
        {
            for id in new.iter() {
                router
                    .wait(id, Some(Duration::from_millis(5_000)))?
                    .metrics(
                        |x| x.current_leader.is_some() && new.contains(&x.current_leader.unwrap()),
                        format!("node {} in new cluster has leader in new cluster, {}", id, mes),
                    )
                    .await?;
            }
        }

        let new_leader = router.leader().expect("expected the cluster to have a leader");
        for id in new.iter() {
            // new leader may already elected and committed a blank log.
            router.wait(id, timeout())?.log_at_least(Some(log_index), format!("new cluster, {}", mes)).await?;

            if new_leader != orig_leader {
                router
                    .wait(id, timeout())?
                    .metrics(
                        |x| x.current_term >= 2,
                        "new cluster has term >= 2 because of new election",
                    )
                    .await?;
            }
        }

        for id in only_in_old.clone() {
            // TODO(xp): There is a chance the older leader quits before replicating 2 membership logs to every node.
            //           Thus a node in old cluster may start electing while a new leader already elected in the new
            //           cluster. Such a node keeps electing but it has less logs thus will never succeed.
            //
            //           Error: timeout after 1s when node 2 only in old, from {0, 1, 2} to {4, 5, 6} .state -> Learner
            //           latest: Metrics{id:2,Candidate, term:7, last_log:Some(109), last_applied:Some(LogId
            //           { leader_id: LeaderId { term: 1, node_id: 0 }, index: 109 }), leader:None,
            //           membership:{log_id:1-0-109 membership:members:[{0, 1, 2},{4, 5, 6}],learners:[]},
            //           snapshot:None, replication:

            router
                .wait(id, timeout())?
                .metrics(
                    |x| x.state == ServerState::Learner || x.state == ServerState::Candidate,
                    format!("node {} only in old, {}", id, mes),
                )
                .await?;
        }
    }

    tracing::info!("--- write another 100 logs");
    {
        // get new leader

        let m = router
            .wait(new.iter().next().unwrap(), timeout())?
            .metrics(|x| x.current_leader.is_some(), format!("wait for new leader, {}", mes))
            .await?;

        let leader = m.current_leader.unwrap();

        router.client_request_many(leader, "client", 100).await;
        log_index += 100;
    }

    for id in new.iter() {
        router
            .wait(id, timeout())?
            // new leader may commit a blonk log
            .log_at_least(Some(log_index), format!("new cluster recv logs 100~200, {}", mes))
            .await?;
    }

    tracing::info!("--- log will not be sync to removed node");
    {
        for id in only_in_old {
            let res = router
                .wait(id, timeout())?
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
    Some(Duration::from_millis(1000))
}
