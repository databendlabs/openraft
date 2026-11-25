//! A learner that joins after the leader purged its logs catches up from a snapshot.

use std::sync::Arc;
use std::time::Duration;

use maplit::btreemap;
use openraft::Config;
use openraft::type_config::TypeConfigExt;
use raft_kv_memstore_grpc::LogStore;
use raft_kv_memstore_grpc::Raft;
use raft_kv_memstore_grpc::StateMachineStore;
use raft_kv_memstore_grpc::TypeConfig;
use raft_kv_memstore_grpc::grpc::raft_service::RaftServiceImpl;
use raft_kv_memstore_grpc::network::Network;
use raft_kv_memstore_grpc::protobuf as pb;
use raft_kv_memstore_grpc::protobuf::raft_service_server::RaftServiceServer;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;

const TIMEOUT: Option<Duration> = Some(Duration::from_secs(10));

#[test]
fn learner_catches_up_from_a_snapshot() -> anyhow::Result<()> {
    TypeConfig::run(async {
        let config = Arc::new(Config::default().validate()?);
        let leader = Raft::new(
            1,
            config.clone(),
            Network {},
            LogStore::default(),
            Arc::new(StateMachineStore::default()),
        )
        .await?;

        tracing::info!("--- node 1 writes more than 1 KiB of state, snapshots it and purges its logs");
        let snapshot_log_id = {
            // Node 1 runs no server: a learner sends no request to the leader.
            let node1 = pb::Node {
                node_id: 1,
                rpc_addr: String::new(),
            };
            leader.initialize(btreemap! {1 => node1}).await?;

            for i in 0..20 {
                let set = pb::SetRequest {
                    key: format!("key-{}", i),
                    value: "v".repeat(100),
                };
                leader.client_write(set).await?;
            }

            leader.trigger().snapshot().await?;
            let metrics = leader.wait(TIMEOUT).metrics(|m| m.snapshot.is_some(), "node 1 builds a snapshot").await?;
            let snapshot_log_id = metrics.snapshot.unwrap();

            leader.trigger().purge_log(snapshot_log_id.index).await?;
            leader.wait(TIMEOUT).purged(Some(snapshot_log_id), "node 1 purges the logs in the snapshot").await?;
            snapshot_log_id
        };

        tracing::info!(%snapshot_log_id, "--- add node 2 as a learner, which can catch up only from the snapshot");
        {
            let incoming = TcpIncoming::bind("127.0.0.1:0".parse()?)?;
            let node2 = pb::Node {
                node_id: 2,
                rpc_addr: incoming.local_addr()?.to_string(),
            };
            let state_machine2 = Arc::new(StateMachineStore::default());
            let raft2 = Raft::new(2, config, Network {}, LogStore::default(), state_machine2.clone()).await?;

            // Limit a message to 1 KiB, as `start_raft_app` does.
            let service = RaftServiceServer::new(RaftServiceImpl::new(raft2.clone())).max_decoding_message_size(1024);
            TypeConfig::spawn(Server::builder().add_service(service).serve_with_incoming(incoming));

            leader.add_learner(2, node2, false).await?;
            raft2.wait(TIMEOUT).snapshot(snapshot_log_id, "node 2 installs the snapshot").await?;

            let state_machine = state_machine2.state_machine.lock().await;
            assert_eq!(Some(&"v".repeat(100)), state_machine.data.get("key-19"));
        }

        Ok(())
    })
}
