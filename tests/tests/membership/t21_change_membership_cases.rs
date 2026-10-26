use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::impls::ChangeMembers;
use openraft_memstore::MemNodeId;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::ut_harness;

// --- change ---

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m0_change_m12() -> anyhow::Result<()> {
    change_from_to(btreeset! {0}, btreeset! {1,2}, 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m0_change_m123() -> anyhow::Result<()> {
    change_from_to(btreeset! {0}, btreeset! {1,2,3}, 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m01_change_m12() -> anyhow::Result<()> {
    change_from_to(btreeset! {0, 1}, btreeset! {1,2}, 1).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m01_change_m1() -> anyhow::Result<()> {
    change_from_to(btreeset! {0, 1}, btreeset! {1}, 1).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m01_change_m2() -> anyhow::Result<()> {
    change_from_to(btreeset! {0, 1}, btreeset! {2}, 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m01_change_m3() -> anyhow::Result<()> {
    change_from_to(btreeset! {0, 1}, btreeset! {3}, 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m012_change_m4() -> anyhow::Result<()> {
    change_from_to(btreeset! {0, 1, 2}, btreeset! {4}, 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m012_change_m456() -> anyhow::Result<()> {
    change_from_to(btreeset! {0, 1, 2}, btreeset! {4,5,6}, 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m01234_change_m0123() -> anyhow::Result<()> {
    change_from_to(btreeset! {0, 1, 2, 3, 4}, btreeset! {0,1,2,3}, 1).await
}

// --- add ---

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m0_add_m01() -> anyhow::Result<()> {
    change_by_add(btreeset! {0}, &[0, 1], 1).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m0_add_m12() -> anyhow::Result<()> {
    change_by_add(btreeset! {0}, &[1, 2], 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m01_add_m() -> anyhow::Result<()> {
    change_by_add(btreeset! {0,1}, &[], 1).await
}

// --- remove ---

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m012_remove_m01() -> anyhow::Result<()> {
    change_by_remove(btreeset! {0,1,2}, &[0, 1], 2).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m012_remove_m3() -> anyhow::Result<()> {
    change_by_remove(btreeset! {0,1,2}, &[3], 1).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m012_remove_m() -> anyhow::Result<()> {
    change_by_remove(btreeset! {0,1,2}, &[], 1).await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn m012_remove_m13() -> anyhow::Result<()> {
    change_by_remove(btreeset! {0,1,2}, &[1, 3], 1).await
}

/// `membership_entries` is 1 when every quorum of `old` intersects every quorum of the new
/// voter set, and 2 when the change needs a joint config first.
#[tracing::instrument(level = "debug")]
async fn change_from_to(
    old: BTreeSet<MemNodeId>,
    change_members: BTreeSet<MemNodeId>,
    membership_entries: u64,
) -> anyhow::Result<()> {
    let new = change_members;

    let mes = format!("from {:?} to {:?}", old, new);

    let only_in_old = old.difference(&new);
    let only_in_new = new.difference(&old);

    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(old.clone(), btreeset! {}).await?;

    tracing::info!(log_index, "--- write 10 logs");
    {
        router.client_request_many(0, "client", 10).await?;
        log_index += 10;

        for id in old.iter() {
            router.wait(id).applied_index(Some(log_index), &format!("write 10 logs, {}", mes)).await?;
        }
    }

    let orig_leader = router.leader().expect("expected the cluster to have a leader");

    tracing::info!(log_index, "--- change to {:?}", new);
    {
        for id in only_in_new {
            router.new_raft_node(*id).await?;
            router.add_learner(0, *id).await?;
            log_index += 1;
            for id in old.iter() {
                router.wait(id).applied_index(Some(log_index), &format!("add learner, {}", mes)).await?;
            }
        }

        let node = router.get_raft_handle(&0)?;
        node.change_membership(new.clone(), false).await?;
        log_index += membership_entries;

        tracing::info!(log_index, "--- wait for old leader or its transfer target");
        {
            for id in new.iter() {
                router
                    .wait_with_timeout(id, Duration::from_millis(5_000))
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
            router.wait(id).applied_index_at_least(Some(log_index), format!("new cluster, {}", mes)).await?;

            if new_leader != orig_leader {
                router
                    .wait(id)
                    .metrics(
                        |x| x.current_term >= 2,
                        "new cluster has term >= 2 because of new election",
                    )
                    .await?;
            }
        }
    }

    tracing::info!(log_index, "--- removed nodes are left in non-leader state");
    {
        for id in only_in_old.clone() {
            router
                .wait(id)
                .metrics(
                    |x| {
                        x.state == ServerState::Follower
                            || x.state == ServerState::Learner
                            || x.state == ServerState::Candidate
                    },
                    format!("node {} only in old, {}", id, mes),
                )
                .await?;
        }
    }

    tracing::info!(log_index, "--- write another 10 logs");
    {
        let m = router
            .wait(new.iter().next().unwrap())
            .metrics(|x| x.current_leader.is_some(), format!("wait for new leader, {}", mes))
            .await?;

        let leader = m.current_leader.unwrap();
        wait_for_leader_lease(&router, leader, &mes).await?;

        router.client_request_many(leader, "client", 10).await?;
        log_index += 10;
    }

    for id in new.iter() {
        router
            .wait(id)
            // new leader may commit a blank log
            .applied_index_at_least(Some(log_index), format!("new cluster recv logs 10~20, {}", mes))
            .await?;
    }

    tracing::info!(log_index, "--- log will not be sync to removed node");
    {
        for id in only_in_old {
            let res = router
                .wait_with_timeout(id, Duration::from_millis(1000))
                .applied_index(
                    Some(log_index),
                    format!("node {} in old cluster wont recv new logs, {}", id, mes),
                )
                .await;
            expect_timeout(res)?;
        }
    }

    Ok(())
}

/// Test change-membership by adding voters.
///
/// `membership_entries` is the same as in [`change_from_to`].
#[tracing::instrument(level = "debug")]
async fn change_by_add(old: BTreeSet<MemNodeId>, add: &[MemNodeId], membership_entries: u64) -> anyhow::Result<()> {
    let change = ChangeMembers::AddVoterIds(add.iter().copied().collect());

    let mes = format!("from {:?} {:?}", old, change);

    let new = old.clone().union(&add.iter().copied().collect()).copied().collect::<BTreeSet<_>>();
    let only_in_new = new.difference(&old);

    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(old.clone(), btreeset! {}).await?;

    tracing::info!(log_index, "--- write 10 logs");
    {
        let written = router.client_request_many(0, "client", 10).await?;
        log_index += written.count;
        for id in old.iter() {
            router.wait(id).applied_index(Some(log_index), format!("write 10 logs, {}", mes)).await?;
        }
    }

    let leader_id = router.leader().expect("expected the cluster to have a leader");

    tracing::info!(log_index, "--- add learner before change-membership");
    {
        for id in only_in_new {
            router.new_raft_node(*id).await?;
            router.add_learner(0, *id).await?;
            log_index += 1;
            router.wait(id).applied_index(Some(log_index), format!("add learner, {}", mes)).await?;
        }
    }

    tracing::info!(log_index, "--- change: {:?}", change);
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(change, false).await?;
        log_index += membership_entries;

        for id in new.iter() {
            router.wait(id).applied_index_at_least(Some(log_index), format!("new cluster, {}", mes)).await?;
        }
    }

    tracing::info!(log_index, "--- write another 10 logs");
    {
        let written = router.client_request_many(leader_id, "client", 10).await?;
        log_index += written.count;

        let mes = format!("new cluster recv logs 10~20, {}", mes);

        for id in new.iter() {
            router.wait(id).applied_index_at_least(Some(log_index), &mes).await?;
        }
    }

    Ok(())
}

/// `membership_entries` is the same as in [`change_from_to`].
#[tracing::instrument(level = "debug")]
async fn change_by_remove(
    old: BTreeSet<MemNodeId>,
    remove: &[MemNodeId],
    membership_entries: u64,
) -> anyhow::Result<()> {
    let change = ChangeMembers::RemoveVoters(remove.iter().copied().collect());

    let mes = format!("from {:?} {:?}", old, change);

    let new = old.clone().difference(&remove.iter().copied().collect()).copied().collect::<BTreeSet<_>>();
    let only_in_old = old.difference(&new);

    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(old.clone(), btreeset! {}).await?;

    tracing::info!(log_index, "--- write 10 logs");
    {
        let written = router.client_request_many(0, "client", 10).await?;
        log_index += written.count;
        for id in old.iter() {
            router.wait(id).applied_index(Some(log_index), format!("write 10 logs, {}", mes)).await?;
        }
    }

    let orig_leader = router.leader().expect("expected the cluster to have a leader");

    tracing::info!(log_index, "--- change {:?}", &change);
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(change.clone(), false).await?;
        log_index += membership_entries;

        tracing::info!(log_index, "--- wait for old leader or its transfer target");
        {
            for id in new.iter() {
                router
                    .wait_with_timeout(id, Duration::from_millis(5_000))
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
            router.wait(id).applied_index_at_least(Some(log_index), format!("new cluster, {}", mes)).await?;

            if new_leader != orig_leader {
                router
                    .wait(id)
                    .metrics(
                        |x| x.current_term >= 2,
                        "new cluster has term >= 2 because of new election",
                    )
                    .await?;
            }
        }
    }

    tracing::info!(log_index, "--- removed nodes are left in follower state");
    {
        for id in only_in_old.clone() {
            // The removed node will be left in follower state, it can never elect itself
            // successfully.
            router
                .wait(id)
                .metrics(
                    |x| x.state != ServerState::Leader,
                    format!("node {} only in old, {}", id, mes),
                )
                .await?;
        }
    }

    tracing::info!(log_index, "--- write another 10 logs");
    {
        let m = router
            .wait(new.iter().next().unwrap())
            .metrics(|x| x.current_leader.is_some(), format!("wait for new leader, {}", mes))
            .await?;

        let leader_id = m.current_leader.unwrap();
        wait_for_leader_lease(&router, leader_id, &mes).await?;

        let written = router.client_request_many(leader_id, "client", 10).await?;
        log_index += written.count;
    }

    for id in new.iter() {
        router
            .wait(id)
            // new leader may commit a blank log
            .applied_index_at_least(Some(log_index), format!("new cluster recv logs 10~20, {}", mes))
            .await?;
    }

    tracing::info!(log_index, "--- log will not be sync to removed node");
    {
        for id in only_in_old {
            let res = router
                .wait_with_timeout(id, Duration::from_millis(1000))
                .applied_index(
                    Some(log_index),
                    format!("node {} in old cluster wont recv new logs, {}", id, mes),
                )
                .await;
            expect_timeout(res)?;
        }
    }

    Ok(())
}

async fn wait_for_leader_lease(router: &RaftRouter, leader_id: MemNodeId, mes: &str) -> anyhow::Result<()> {
    router
        .wait(&leader_id)
        .leader_with_quorum_acked(None, format!("leader establishes its quorum lease, {}", mes))
        .await?;

    Ok(())
}
