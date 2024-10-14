use std::backtrace::Backtrace;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::panic::PanicHookInfo;
use std::time::Duration;

use maplit::btreemap;
use maplit::btreeset;
use openraft::error::Infallible;
use openraft::BasicNode;
use raft_kv_memstore_singlethreaded::router::Router;
use raft_kv_memstore_singlethreaded::start_raft;
use raft_kv_memstore_singlethreaded::store::Request;
use raft_kv_memstore_singlethreaded::typ::CheckIsLeaderError;
use raft_kv_memstore_singlethreaded::typ::ClientWriteError;
use raft_kv_memstore_singlethreaded::typ::ClientWriteResponse;
use raft_kv_memstore_singlethreaded::typ::InitializeError;
use raft_kv_memstore_singlethreaded::typ::RaftMetrics;
use raft_kv_memstore_singlethreaded::NodeId;
use tokio::task;
use tokio::task::LocalSet;
use tracing_subscriber::EnvFilter;

pub fn log_panic(panic: &PanicHookInfo) {
    let backtrace = format!("{:?}", Backtrace::force_capture());

    eprintln!("{}", panic);

    if let Some(location) = panic.location() {
        tracing::error!(
            message = %panic,
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
        eprintln!("{}:{}:{}", location.file(), location.line(), location.column());
    } else {
        tracing::error!(message = %panic, backtrace = %backtrace);
    }

    eprintln!("{}", backtrace);
}

/// Setup a cluster of 3 nodes.
/// Write to it and read from it.
#[tokio::test]
async fn test_cluster() {
    std::panic::set_hook(Box::new(|panic| {
        log_panic(panic);
    }));

    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let router = Router::default();

    let local = LocalSet::new();

    local
        .run_until(async move {
            task::spawn_local(start_raft(NodeId::new(1), router.clone()));
            task::spawn_local(start_raft(NodeId::new(2), router.clone()));
            task::spawn_local(start_raft(NodeId::new(3), router.clone()));

            run_test(router).await;
        })
        .await;
}

async fn run_test(router: Router) {
    // Wait for server to start up.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // --- 1. Initialize the target node as a cluster of only one node.
    //        After init(), the single node cluster will be fully functional.

    println!("=== init single node cluster");
    router.send::<(), (), InitializeError>(NodeId::new(1), "/mng/init", ()).await.unwrap();

    println!("=== metrics after init");
    let metrics = router.send::<(), RaftMetrics, Infallible>(NodeId::new(1), "/mng/metrics", ()).await.unwrap();
    println!("metrics: {:#?}", metrics);

    // --- 2. Add node 2 and 3 to the cluster as `Learner`, to let them start to receive log replication
    // from the        leader.

    println!("=== add-learner 2");
    let resp = router
        .send::<NodeId, ClientWriteResponse, ClientWriteError>(NodeId::new(1), "/mng/add-learner", NodeId::new(2))
        .await
        .unwrap();
    println!("add-learner-2 resp: {:#?}", resp);

    println!("=== add-learner 3");
    let resp = router
        .send::<NodeId, ClientWriteResponse, ClientWriteError>(NodeId::new(1), "/mng/add-learner", NodeId::new(3))
        .await
        .unwrap();
    println!("add-learner-3 resp: {:#?}", resp);

    println!("=== metrics after add-learner");
    let metrics = router.send::<(), RaftMetrics, Infallible>(NodeId::new(1), "/mng/metrics", ()).await.unwrap();
    println!("metrics: {:#?}", metrics);

    assert_eq!(
        &vec![btreeset![NodeId::new(1)]],
        metrics.membership_config.membership().get_joint_config()
    );

    let nodes_in_cluster = metrics
        .membership_config
        .nodes()
        .map(|(nid, node)| (*nid, node.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        btreemap! {
            NodeId::new(1) => BasicNode::new(""),
            NodeId::new(2) => BasicNode::new(""),
            NodeId::new(3) => BasicNode::new(""),
        },
        nodes_in_cluster
    );

    // --- 3. Turn the two learners to members. A member node can vote or elect itself as leader.

    println!("=== change-membership to 1,2,3");
    let resp = router
        .send::<BTreeSet<NodeId>, ClientWriteResponse, ClientWriteError>(
            NodeId::new(1),
            "/mng/change-membership",
            btreeset![NodeId::new(1), NodeId::new(2), NodeId::new(3),],
        )
        .await
        .unwrap();
    println!("change-membership resp: {:#?}", resp);

    // --- After change-membership, some cluster state will be seen in the metrics.
    //
    // ```text
    // metrics: RaftMetrics {
    //   current_leader: Some(1),
    //   membership_config: EffectiveMembership {
    //        log_id: LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 8 },
    //        membership: Membership { learners: {}, configs: [{1, 2, 3}] }
    //   },
    //   leader_metrics: Some(LeaderMetrics { replication: {
    //     2: ReplicationMetrics { matched: Some(LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 7 }) },
    //     3: ReplicationMetrics { matched: Some(LogId { leader_id: LeaderId { term: 1, node_id: 1 }, index: 8 }) }} })
    // }
    // ```

    println!("=== metrics after change-member");
    let metrics = router.send::<(), RaftMetrics, Infallible>(NodeId::new(1), "/mng/metrics", ()).await.unwrap();
    println!("metrics: {:#?}", metrics);
    assert_eq!(
        &vec![btreeset![NodeId::new(1), NodeId::new(2), NodeId::new(3)]],
        metrics.membership_config.membership().get_joint_config()
    );

    // --- Try to write some application data through the leader.

    println!("=== write `foo=bar`");
    let resp = router
        .send::<Request, ClientWriteResponse, ClientWriteError>(
            NodeId::new(1),
            "/app/write",
            Request::set("foo", "bar"),
        )
        .await
        .unwrap();
    println!("write resp: {:#?}", resp);

    // --- Wait for a while to let the replication get done.

    tokio::time::sleep(Duration::from_millis(1_000)).await;

    // --- Read it

    println!("=== read `foo` on node 1");
    let resp = router
        .send::<String, String, CheckIsLeaderError>(NodeId::new(1), "/app/read", "foo".to_string())
        .await
        .unwrap();
    println!("read resp: {:#?}", resp);
    assert_eq!("bar", resp);
}
