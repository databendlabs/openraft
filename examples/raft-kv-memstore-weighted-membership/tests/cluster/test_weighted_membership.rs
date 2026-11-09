use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use app_http::AddLearnerRequest;
use app_http::Client;
use openraft::LogIdOptionExt;
use openraft::LogIndexOptionExt;
use openraft::NodeInfo;
use openraft::errors::ClientWriteError;
use openraft::errors::InitializeError;
use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft::raft::ClientWriteResponse;
use openraft::type_config::TypeConfigExt;
use openraft_membership::WeightedMembership;
use raft_kv_memstore_weighted_membership::TypeConfig;
use raft_kv_memstore_weighted_membership::membership_api::ChangeMembershipRequest;
use raft_kv_memstore_weighted_membership::membership_api::InitRequest;
use types_kv::Request;
use types_kv::Response;

use crate::nodes;
use crate::nodes::Nodes;
use crate::nodes::api_addr;
use crate::nodes::raft_addr;

/// Give 4 voters the weights `{1:2, 2:1, 3:1, 4:1}` with write weight 3, then stop voters 3 and
/// 4: voters 1 and 2 weigh 3 together, so they keep committing writes.
#[test]
fn test_weighted_membership() -> anyhow::Result<()> {
    TypeConfig::run(async {
        let res = tokio::time::timeout(Duration::from_secs(60), test_weighted_membership_inner()).await;
        res.context("the test did not finish within 60 s")?
    })
}

async fn test_weighted_membership_inner() -> anyhow::Result<()> {
    let mut nodes = Nodes::start([1, 2, 3, 4]).await?;
    let leader = Client::<TypeConfig>::new(1, api_addr(1));

    println!("=== init node 1 alone, with weight 1 and write weight 1");
    {
        let req = InitRequest {
            weights: BTreeMap::from([(1, 1)]),
            write_weight: 1,
            nodes: BTreeMap::new(),
        };
        let res: Result<(), InitializeError<TypeConfig>> = nodes::post(1, "init", &req).await?;
        res?;

        nodes::wait("node 1 leads and applies its blank entry", async || {
            let metrics = leader.metrics().await?;
            Ok(metrics.current_leader == Some(1) && metrics.last_applied.index() >= Some(1))
        })
        .await?;
    }

    println!("=== add nodes 2, 3 and 4 as learners");
    for id in [2, 3, 4] {
        let req = AddLearnerRequest {
            node_id: id,
            api_addr: api_addr(id),
            raft_addr: raft_addr(id),
        };
        let response = leader.add_learner(&req).await??;
        let metrics = leader.metrics().await?;
        assert!(metrics.membership_config.log_id().as_ref() >= Some(&response.log_id));
        let matched = metrics.replication.as_ref().and_then(|replication| replication.get(&id));
        let matched = matched.context("the blocking learner API must observe replication progress")?;
        let lag = metrics.last_log_index.next_index().saturating_sub(matched.index().next_index());
        assert!(lag <= openraft::Config::default().replication_lag_threshold);
    }

    println!("=== give voter 1 weight 2 and voters 2, 3 and 4 weight 1, with write weight 3");
    let membership = {
        let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
        let req = ChangeMembershipRequest {
            weights: weights.clone(),
            write_weight: Some(3),
        };
        let res: Result<ClientWriteResponse<TypeConfig>, ClientWriteError<TypeConfig>> =
            nodes::post(1, "change-membership", &req).await?;
        let resp = res?;

        let nodes: BTreeMap<u64, NodeInfo> =
            (1..=4).map(|id| (id, NodeInfo::new(raft_addr(id), api_addr(id)))).collect();
        let want = WeightedMembership::new(weights, 3, nodes)?;
        assert_eq!(Some(&want), resp.membership.as_ref());

        println!("=== the change moved through a joint config: log 5 is the joint config, log 6 the new one");
        assert_eq!(6, resp.log_id.index());
        want
    };

    println!("=== voters 1 and 2 weigh 3, a read quorum and a write quorum; voters 3 and 4 weigh 2, neither");
    {
        let read = membership.read_quorum_set();
        let write = membership.write_quorum_set();
        assert!(read.is_quorum([1, 2].iter()) && write.is_quorum([1, 2].iter()));
        assert!(!read.is_quorum([3, 4].iter()) && !write.is_quorum([3, 4].iter()));
    }

    println!("=== write foo=1, and read it on every node");
    {
        let resp = leader.write(&Request::set("foo", "1")).await??;
        let want = Response::new("1", resp.log_id.index());
        assert_eq!(want, resp.data);

        for id in [1, 2, 3, 4] {
            let client = Client::<TypeConfig>::new(id, api_addr(id));
            let what = format!("node {} reads foo=1", id);
            nodes::wait(&what, async || Ok(client.read(&"foo".to_string()).await? == want)).await?;
        }
    }

    println!("=== stop voters 3 and 4");
    {
        nodes.kill(3)?;
        nodes.kill(4)?;
    }

    println!("=== write foo=2: voters 1 and 2 weigh 3, which reaches the write weight");
    {
        let resp = leader.write(&Request::set("foo", "2")).await??;
        let want = Response::new("2", resp.log_id.index());
        assert_eq!(want, resp.data);

        let follower = Client::<TypeConfig>::new(2, api_addr(2));
        let what = "node 2 reads foo=2";
        nodes::wait(what, async || Ok(follower.read(&"foo".to_string()).await? == want)).await?;

        println!("=== a linearizable read confirms the leadership with voters 1 and 2");
        let got = leader.linearizable_read(&"foo".to_string()).await??;
        assert_eq!(want, got);
    }

    Ok(())
}
