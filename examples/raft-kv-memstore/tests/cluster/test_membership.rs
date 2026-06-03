use std::collections::BTreeMap;
use std::time::Duration;

use app_http::AddLearnerRequest;
use app_http::Client;
use maplit::btreemap;
use maplit::btreeset;
use openraft::NodeInfo as Node;
use openraft::type_config::TypeConfigExt;
use raft_kv_memstore::TypeConfig;

use crate::util;

/// Distinct port range so this test never collides with the others run in parallel.
const PORT_BASE: u16 = 23000;

/// Membership management: grow a single node into `{1, 2, 3}`, then shrink to `{3}`.
#[test]
fn test_membership() {
    TypeConfig::run(test_membership_inner()).unwrap();
}

async fn test_membership_inner() -> anyhow::Result<()> {
    util::spawn_nodes(PORT_BASE);
    TypeConfig::sleep(Duration::from_millis(1_000)).await;

    let leader = Client::<TypeConfig>::new(1, util::api_addr(PORT_BASE, 1));

    // --- Initialize node 1 as a single-voter cluster.
    println!("=== init single-node cluster {{1}}");
    leader.init().await??;
    TypeConfig::sleep(Duration::from_millis(200)).await;

    let metrics = leader.metrics().await?;
    assert_eq!(
        &vec![btreeset![1]],
        metrics.membership_config.membership().get_joint_config()
    );

    // --- Add node 2 and 3 as learners. They receive replication but cannot vote yet.
    println!("=== add node 2 and 3 as learners");
    leader
        .add_learner(&AddLearnerRequest {
            node_id: 2,
            api_addr: util::api_addr(PORT_BASE, 2),
            raft_addr: util::raft_addr(PORT_BASE, 2),
        })
        .await??;
    leader
        .add_learner(&AddLearnerRequest {
            node_id: 3,
            api_addr: util::api_addr(PORT_BASE, 3),
            raft_addr: util::raft_addr(PORT_BASE, 3),
        })
        .await??;

    // The learners are known to the cluster, but the voter set is still `{1}`.
    let metrics = leader.metrics().await?;
    assert_eq!(
        &vec![btreeset![1]],
        metrics.membership_config.membership().get_joint_config()
    );
    let nodes = metrics
        .membership_config
        .nodes()
        .map(|(nid, node)| (*nid, node.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        btreemap! {
            1 => Node::new(util::raft_addr(PORT_BASE, 1), util::api_addr(PORT_BASE, 1)),
            2 => Node::new(util::raft_addr(PORT_BASE, 2), util::api_addr(PORT_BASE, 2)),
            3 => Node::new(util::raft_addr(PORT_BASE, 3), util::api_addr(PORT_BASE, 3)),
        },
        nodes
    );

    // --- Promote the learners to voters.
    println!("=== promote learners to voters {{1, 2, 3}}");
    leader.change_membership(&btreeset! {1, 2, 3}).await??;
    let metrics = leader.metrics().await?;
    assert_eq!(
        &vec![btreeset![1, 2, 3]],
        metrics.membership_config.membership().get_joint_config()
    );

    // --- Shrink the cluster back to a single voter `{3}`.
    println!("=== shrink membership to {{3}}");
    leader.change_membership(&btreeset! {3}).await??;
    TypeConfig::sleep(Duration::from_millis(8_000)).await;

    let metrics = leader.metrics().await?;
    assert_eq!(
        &vec![btreeset![3]],
        metrics.membership_config.membership().get_joint_config()
    );

    Ok(())
}
