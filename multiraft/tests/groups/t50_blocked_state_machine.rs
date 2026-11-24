use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use openraft::Config;
use openraft::RaftMetrics;
use openraft::ReadPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft_memstore::BlockOperation;
use openraft_memstore::ClientRequest;
use openraft_memstore::new_mem_store;
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::C;
use crate::Raft;
use crate::group_network::GroupNetwork;
use crate::local_members::LocalMembers;

/// The groups in the process.
const GROUPS: u64 = 4;

/// The members of each group, with node ids from 1 up.
const MEMBERS: u64 = 3;

/// The writes that each group receives.
const WRITES: u64 = 100;

/// How long an operation may take when nothing holds it back.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A blocked state machine holds back only its own group.
///
/// The leader of group 0 holds a snapshot build at a memstore gate, and an external function holds
/// its state-machine worker. Meanwhile the other groups of the process must finish their writes, a
/// membership change, a snapshot and a linearizable read. Once released, group 0 must answer every
/// write it received, and every member of every group must apply the writes of its group.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_blocked_state_machine_holds_back_only_its_group() -> Result<()> {
    crate::init_tracing();

    let config = Config {
        enable_elect: false,
        ..Default::default()
    };
    let config = Arc::new(config.validate()?);

    tracing::info!(
        groups = GROUPS,
        members = MEMBERS,
        "--- create the groups; node 1 of each initializes it and, with elections off, stays its leader"
    );
    let (rafts, state_machines) = {
        let router = LocalMembers::default();
        let mut rafts = Vec::new();
        let mut state_machines = Vec::new();
        for group_id in 0..GROUPS {
            let mut group_rafts = Vec::new();
            let mut group_state_machines = Vec::new();
            for node_id in 1..=MEMBERS {
                let (log_store, state_machine) = new_mem_store();
                let network = GroupNetwork::new(router.clone(), group_id);
                let raft = Raft::new(node_id, config.clone(), network, log_store, state_machine.clone()).await?;
                group_rafts.push(raft);
                group_state_machines.push(state_machine);
            }
            rafts.push(group_rafts);
            state_machines.push(group_state_machines);
        }
        router.set_members(&rafts);

        for leader in rafts.iter().map(|group| &group[0]) {
            leader.initialize(BTreeSet::from_iter(1..=MEMBERS)).await?;
            leader.wait(Some(TIMEOUT)).applied_index(Some(1), "the leader applies its blank log").await?;
        }
        (rafts, state_machines)
    };
    let blocked = &rafts[0][0];

    tracing::info!("--- hold a snapshot build of the leader of group 0, then its state-machine worker");
    let (build_gate, release_worker) = {
        let build_gate = state_machines[0][0].block.pause(BlockOperation::BuildSnapshot);
        blocked.trigger().snapshot().await?;
        timeout(TIMEOUT, build_gate.entered()).await.context("the snapshot build reaches its gate")?;

        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        blocked
            .external_state_machine_request(move |_sm| {
                Box::pin(async move {
                    started_tx.send(()).ok();
                    release_rx.await.ok();
                })
            })
            .await?;
        timeout(TIMEOUT, started_rx).await.context("the worker runs the external function")??;
        (build_gate, release_tx)
    };

    tracing::info!(
        writes = WRITES,
        "--- write to group 0: the group commits every write, and its leader applies none"
    );
    let pending = {
        let mut pending = Vec::new();
        for serial in 0..WRITES {
            let leader = blocked.clone();
            let request = write_request(0, serial);
            pending.push(tokio::spawn(async move { leader.client_write(request).await }));
        }

        let last = 1 + WRITES;
        blocked.wait(Some(TIMEOUT)).committed_index(Some(last), "group 0 commits every write").await?;
        pending
    };

    tracing::info!(
        writes = WRITES,
        "--- while group 0 is blocked, groups 1 to 3 write, make node 3 a learner, build a snapshot and read"
    );
    for (group_id, group) in (1u64..).zip(&rafts[1..]) {
        let leader = &group[0];
        let work = async {
            for serial in 0..WRITES {
                leader.client_write(write_request(group_id, serial)).await?;
            }

            let response = leader.change_membership([1, 2], true).await?;
            let last = response.log_id.index();
            assert_eq!(1 + WRITES + 1, last, "the membership change appends one entry");

            leader.trigger().snapshot().await?;
            let built = |m: &RaftMetrics<C>| m.snapshot.as_ref().map(|log_id| log_id.index()) == Some(last);
            leader.wait(None).metrics(built, "the leader builds a snapshot").await?;

            let read = leader.ensure_linearizable(ReadPolicy::ReadIndex).await?;
            assert_eq!(last, read.index(), "the read covers the last entry");
            anyhow::Ok(())
        };
        timeout(TIMEOUT, work).await.with_context(|| format!("group {group_id} is held back"))??;
    }

    tracing::info!("--- group 0 is still blocked: no write answered, nothing applied, no snapshot built");
    {
        let answered = pending.iter().filter(|write| write.is_finished()).count();
        assert_eq!(0, answered, "a blocked leader answers no write");

        let metrics = blocked.metrics().borrow_watched().clone();
        assert_eq!(Some(1), metrics.last_applied.map(|log_id| log_id.index()));
        assert_eq!(None, metrics.snapshot);
    }

    tracing::info!("--- release group 0: its leader answers every write, and finishes the snapshot it held");
    {
        build_gate.release();
        drop(release_worker);

        let mut answered = BTreeSet::new();
        for write in pending {
            let joined = timeout(TIMEOUT, write).await.context("group 0 answers a write")?;
            let response = joined??;
            answered.insert(response.log_id.index());
        }
        assert_eq!(
            BTreeSet::from_iter(2..=1 + WRITES),
            answered,
            "group 0 answers each write once"
        );

        let built = |m: &RaftMetrics<C>| m.snapshot.as_ref().map(|log_id| log_id.index()) == Some(1);
        blocked.wait(Some(TIMEOUT)).metrics(built, "group 0 builds the snapshot it held").await?;
    }

    tracing::info!("--- every member of every group applies the writes of its group");
    for (group_id, (group, group_state_machines)) in (0u64..).zip(rafts.iter().zip(&state_machines)) {
        // Groups 1 to 3 appended a membership entry after their writes.
        let last = if group_id == 0 { 1 + WRITES } else { 1 + WRITES + 1 };
        let expected = (0..WRITES)
            .map(|serial| {
                let request = write_request(group_id, serial);
                (request.client, request.status)
            })
            .collect::<HashMap<_, _>>();

        for (raft, state_machine) in group.iter().zip(group_state_machines) {
            let msg = format!("group {group_id} applies its writes");
            raft.wait(Some(TIMEOUT)).applied_index(Some(last), msg).await?;

            let applied = state_machine.get_state_machine().await.client_status;
            assert_eq!(expected, applied, "the state machine of a member of group {group_id}");
        }
    }

    Ok(())
}

/// A write to group `group_id`, whose `client` key no other write in the process uses.
fn write_request(group_id: u64, serial: u64) -> ClientRequest {
    ClientRequest {
        client: format!("group-{group_id}-write-{serial}"),
        serial,
        status: format!("status-{serial}"),
    }
}
