mod archive_buffer;
mod cluster;
mod codec;
mod command;
mod connection;
mod entry;
mod network;
mod payload;
mod state;
mod state_machine;

use openraft::ReadPolicy;

use crate::archive_buffer::ArchiveBuffer;
use crate::cluster::Cluster;
use crate::entry::Entry;
use crate::payload::Payload;
use crate::state_machine::StateMachine;

type CommittedLeaderId = openraft::impls::leader_id_adv::LeaderId<u64, u64>;
type LogId = openraft::impls::LogId<CommittedLeaderId>;
type Membership = openraft::impls::Membership<u64, openraft::impls::EmptyNode>;
type StoredMembership = openraft::StoredMembership<CommittedLeaderId, Membership>;
type SnapshotMeta = openraft::SnapshotMeta<CommittedLeaderId, Membership>;
type Snapshot = openraft::Snapshot<CommittedLeaderId, Membership, std::io::Cursor<Vec<u8>>>;
type LogStore = log_mem::LogStore<TypeConfig>;
type Raft = openraft::Raft<TypeConfig, StateMachine>;

openraft::declare_raft_types!(
    TypeConfig:
        D = ArchiveBuffer,
        R = (),
        Node = openraft::impls::EmptyNode,
        Payload = Payload,
        Entry = Entry,
);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cluster = Cluster::new().await?;
    let leader = &cluster.nodes[&1].0;
    let response = leader.client_write(ArchiveBuffer::new("hello", "archive-backed world")?).await?;
    cluster.wait_applied(response.log_id.index()).await?;
    leader.ensure_linearizable(ReadPolicy::ReadIndex).await?;
    for (id, (_, state, _)) in &cluster.nodes {
        let value = state.get("hello").await.expect("committed value");
        println!("node {id}: hello = {}", value.archived().value.as_str());
    }
    cluster.shutdown().await
}

#[cfg(test)]
#[path = "main/main_test.rs"]
mod main_test;
