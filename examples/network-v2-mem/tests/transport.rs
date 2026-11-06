//! Runs Raft nodes over the in-process transport, with the KV stores of the other examples.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use log_mem::LogStore;
use network_v2_mem::Registry;
use openraft::Config;
use openraft::Raft;
use openraft::RaftNetworkFactory;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::RPCError;
use openraft::impls::Vote;
use openraft::network::RPCOption;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::VoteRequest;
use openraft::type_config::TypeConfigExt;
use sm_mem::StateMachineStore;
use types_kv::Request;
use types_kv::VersionedValue;

openraft::declare_raft_types!(
    TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
);

type StateMachine = StateMachineStore<TypeConfig>;

/// A started node: its Raft handle and its state machine.
struct Node {
    raft: Raft<TypeConfig, StateMachine>,
    state_machine: StateMachine,
}

const TIMEOUT: Option<Duration> = Some(Duration::from_secs(5));

/// Starts a node with empty stores for each of `ids`, and registers it with `registry`.
///
/// Ticks are off, so only the test starts elections and heartbeats. The election timeout, which is
/// also the leader lease, outlasts every test, so the leader accepts writes without heartbeats.
async fn start(
    registry: &Arc<Registry<TypeConfig, StateMachine>>,
    ids: impl IntoIterator<Item = u64>,
) -> anyhow::Result<BTreeMap<u64, Node>> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        snapshot_policy: SnapshotPolicy::Never,
        election_timeout_min: 30_000,
        election_timeout_max: 60_000,
        ..Default::default()
    };
    let config = Arc::new(config.validate()?);

    let mut nodes = BTreeMap::new();
    for id in ids {
        let state_machine = StateMachine::default();
        let network = registry.network_factory(id);
        let raft = Raft::new(id, config.clone(), network, LogStore::default(), state_machine.clone()).await?;
        registry.register(id, raft.clone());
        nodes.insert(id, Node { raft, state_machine });
    }
    Ok(nodes)
}

/// A Pre-Vote request from candidate `id`. Handling one changes no state on the target.
fn pre_vote_request(id: u64) -> VoteRequest<TypeConfig> {
    VoteRequest::new(Vote::new(1, id), None)
}

fn rpc_option() -> RPCOption {
    RPCOption::new(Duration::from_secs(1))
}

/// Elections and replication run through the transport, and an isolated node misses writes in both
/// directions until it is healed.
#[test]
fn isolated_node_catches_up_after_heal() -> anyhow::Result<()> {
    TypeConfig::run(async {
        let registry = Arc::new(Registry::default());
        let nodes = start(&registry, [1, 2, 3]).await?;
        let n1 = &nodes[&1].raft;

        tracing::info!("--- initialize node 1 with voters {{1,2,3}}: its election and blank entry need RPCs");
        {
            n1.initialize(BTreeMap::from([(1, ()), (2, ()), (3, ())])).await?;
            for node in nodes.values() {
                node.raft.wait(TIMEOUT).applied_index(Some(1), "blank entry applied").await?;
            }
        }

        tracing::info!("--- a Pre-Vote from node 3 reaches node 1, which answers with its own vote");
        {
            let mut from_3 = registry.network_factory(3).new_client(1, &()).await;
            let resp = from_3.pre_vote(pre_vote_request(3), rpc_option()).await?;
            let leader_vote = n1.metrics().borrow_watched().vote;
            assert_eq!(leader_vote, resp.vote);
        }

        tracing::info!("--- isolate node 3: RPCs from it and to it fail");
        {
            registry.isolate([3]);

            let mut from_3 = registry.network_factory(3).new_client(1, &()).await;
            let res = from_3.pre_vote(pre_vote_request(3), rpc_option()).await;
            assert!(matches!(res, Err(RPCError::Unreachable(_))), "got {:?}", res);

            let mut to_3 = registry.network_factory(1).new_client(3, &()).await;
            let res = to_3.pre_vote(pre_vote_request(1), rpc_option()).await;
            assert!(matches!(res, Err(RPCError::Unreachable(_))), "got {:?}", res);
        }

        tracing::info!("--- nodes 1 and 2 commit a write that node 3 does not receive");
        let log_id = {
            let resp = n1.client_write(Request::set("k", "v")).await?;
            for id in [1, 2] {
                nodes[&id].raft.wait(TIMEOUT).applied_index(Some(resp.log_id.index), "write applied").await?;
            }

            let n3_applied = nodes[&3].raft.metrics().borrow_watched().last_applied;
            assert!(n3_applied < Some(resp.log_id), "node 3 applied {:?}", n3_applied);
            assert_eq!(None, nodes[&3].state_machine.get("k").await);
            resp.log_id
        };

        tracing::info!(%log_id, "--- heal node 3, and wake the replication to it");
        {
            let healed = registry.heal();
            assert_eq!(vec![3], healed.into_iter().collect::<Vec<_>>());
            n1.trigger().reset_backoff([3]).await?;
            n1.trigger().heartbeat().await?;

            nodes[&3].raft.wait(TIMEOUT).applied_index(Some(log_id.index), "write applied after heal").await?;
            let want = Some(VersionedValue {
                value: "v".to_string(),
                version: log_id.index,
            });
            assert_eq!(want, nodes[&3].state_machine.get("k").await);
        }

        for node in nodes.values() {
            node.raft.shutdown().await?;
        }
        Ok(())
    })
}

/// A learner that the purged log cannot catch up receives a snapshot through `full_snapshot`, and
/// a leadership transfer reaches its target through `transfer_leader`.
#[test]
fn forwards_snapshot_and_leadership_transfer() -> anyhow::Result<()> {
    TypeConfig::run(async {
        let registry = Arc::new(Registry::default());
        let nodes = start(&registry, [1, 2, 3, 4]).await?;
        let n1 = &nodes[&1].raft;

        tracing::info!("--- initialize voters {{1,2,3}} and write one entry");
        let log_id = {
            n1.initialize(BTreeMap::from([(1, ()), (2, ()), (3, ())])).await?;
            n1.wait(TIMEOUT).applied_index(Some(1), "blank entry applied").await?;
            let resp = n1.client_write(Request::set("k", "v")).await?;
            resp.log_id
        };

        tracing::info!(%log_id, "--- build a snapshot on node 1 and purge the logs it holds");
        {
            n1.trigger().snapshot().await?;
            n1.wait(TIMEOUT).snapshot(log_id, "snapshot built").await?;
            n1.trigger().purge_log(log_id.index).await?;
            n1.wait(TIMEOUT).purged(Some(log_id), "logs purged").await?;
        }

        tracing::info!("--- add node 4 as a learner: only the snapshot holds the purged entries");
        {
            let resp = n1.add_learner(4, (), true).await?;
            let n4 = &nodes[&4];
            n4.raft.wait(TIMEOUT).applied_index(Some(resp.log_id.index), "learner caught up").await?;

            let n4_snapshot = n4.raft.metrics().borrow_watched().snapshot;
            assert_eq!(Some(log_id), n4_snapshot);
            let want = Some(VersionedValue {
                value: "v".to_string(),
                version: log_id.index,
            });
            assert_eq!(want, n4.state_machine.get("k").await);
        }

        tracing::info!("--- transfer leadership from node 1 to node 2");
        {
            n1.trigger().transfer_leader(2).await?;
            nodes[&2].raft.wait(TIMEOUT).state(ServerState::Leader, "node 2 takes over").await?;
            for id in [1, 3] {
                nodes[&id].raft.wait(TIMEOUT).current_leader(2, "follows node 2").await?;
            }
        }

        for node in nodes.values() {
            node.raft.shutdown().await?;
        }
        Ok(())
    })
}

/// An RPC to a node that is not registered, or through a dropped registry, fails with
/// `Unreachable`.
#[test]
fn unknown_target_and_dropped_registry_are_unreachable() -> anyhow::Result<()> {
    TypeConfig::run(async {
        let registry = Arc::new(Registry::<TypeConfig, StateMachine>::default());
        let mut to_2 = registry.network_factory(1).new_client(2, &()).await;

        let res = to_2.pre_vote(pre_vote_request(1), rpc_option()).await;
        assert!(matches!(res, Err(RPCError::Unreachable(_))), "got {:?}", res);

        drop(registry);
        let res = to_2.pre_vote(pre_vote_request(1), rpc_option()).await;
        assert!(matches!(res, Err(RPCError::Unreachable(_))), "got {:?}", res);
        Ok(())
    })
}
