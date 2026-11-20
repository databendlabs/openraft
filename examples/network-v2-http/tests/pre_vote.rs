//! A pre-vote sent through this transport reaches the target's `Raft::pre_vote()`.
//!
//! `RaftNetworkV2::pre_vote()` grants by default without contacting the target, so a transport
//! that leaves it unimplemented makes Pre-Vote a no-op. The candidate here asks a leader whose log
//! is ahead of its own: a real Pre-Vote round refuses it, and the default implementation would
//! grant it.

use std::collections::BTreeMap;
use std::error::Error;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use log_mem::LogStore;
use network_v2_http::NetworkFactory;
use network_v2_http::Server;
use openraft::Config;
use openraft::NodeInfo;
use openraft::Raft;
use openraft::Vote;
use openraft::async_runtime::WatchReceiver;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::VoteRequest;
use sm_mem::StateMachineStore;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = NodeInfo,
);

#[tokio::test]
async fn pre_vote_reaches_the_target() -> Result<(), Box<dyn Error>> {
    let raft_addr = {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.local_addr()?.to_string()
    };
    let node = NodeInfo {
        raft_addr: raft_addr.clone(),
        data: String::new(),
    };

    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let raft = Raft::<TypeConfig, _>::new(
        1,
        Arc::new(config.validate()?),
        NetworkFactory::new(),
        LogStore::<TypeConfig>::default(),
        StateMachineStore::<TypeConfig>::default(),
    )
    .await?;
    tokio::spawn(Server::new(raft.clone()).run(raft_addr));

    raft.initialize(BTreeMap::from([(1, node.clone())])).await?;
    raft.wait(Some(Duration::from_secs(5))).current_leader(1, "node 1 leads").await?;
    let term = raft.metrics().borrow_watched().current_term;

    let mut factory = NetworkFactory::new();
    let mut client = RaftNetworkFactory::<TypeConfig>::new_client(&mut factory, 1, &node).await;

    // Node 2 asks for the next term without any log, while node 1 has the initial logs.
    let request = VoteRequest::<TypeConfig>::new(Vote::new(term + 1, 2), None);
    let option = RPCOption::new(Duration::from_secs(5));
    let response = client.pre_vote(request, option).await?;

    assert!(
        !response.vote_granted,
        "node 1 must refuse a candidate whose log is behind"
    );
    assert_eq!(
        term,
        raft.metrics().borrow_watched().current_term,
        "a pre-vote changes no term"
    );

    raft.shutdown().await?;
    Ok(())
}
