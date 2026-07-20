use std::time::Duration;

use app_http::Client;
use openraft::type_config::TypeConfigExt;
use raft_kv_memstore::TypeConfig;

use crate::util;

/// Distinct port range so this test never collides with the others run in parallel.
const PORT_BASE: u16 = 25000;

/// The three read modes: local `/read`, leader `/linearizable_read`, and
/// `/follower_read` served by a follower.
#[test]
fn test_read_modes() {
    TypeConfig::run(test_read_modes_inner()).unwrap();
}

async fn test_read_modes_inner() -> anyhow::Result<()> {
    let leader = util::bootstrap(PORT_BASE).await?;

    println!("=== write foo=bar on the leader");
    let write_response = leader
        .write(&types_kv::Request::Set {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .await??;
    let expected = write_response.data;
    TypeConfig::sleep(Duration::from_millis(1_000)).await;

    let follower2 = Client::<TypeConfig>::new(2, util::api_addr(PORT_BASE, 2));
    let follower3 = Client::<TypeConfig>::new(3, util::api_addr(PORT_BASE, 3));

    // 1. Local read: any node answers from its own state machine (may be stale).
    println!("=== /read on a follower");
    assert_eq!(expected, follower2.read(&"foo".to_string()).await?);

    // 2. Linearizable read: only the leader can serve it.
    println!("=== /linearizable_read on the leader");
    assert_eq!(expected, leader.linearizable_read(&"foo".to_string()).await??);

    println!("=== /linearizable_read on a follower must forward to the leader");
    match follower2.linearizable_read(&"foo".to_string()).await? {
        Err(e) => {
            let expect = format!(
                "has to forward request to: Some(1), Some(NodeInfo {{ raft_addr: \"{}\", data: \"{}\" }})",
                util::raft_addr(PORT_BASE, 1),
                util::api_addr(PORT_BASE, 1),
            );
            assert_eq!(expect, e.to_string());
        }
        Ok(_) => panic!("linearizable_read on a follower must return an error"),
    }

    // 3. Follower read: a follower serves a linearizable read by first syncing to the leader's read
    //    index.
    println!("=== /follower_read on the followers");
    assert_eq!(expected, follower2.follower_read(&"foo".to_string()).await??);
    assert_eq!(expected, follower3.follower_read(&"foo".to_string()).await??);

    // A missing key has no versioned value.
    println!("=== /follower_read of a missing key");
    assert_eq!(
        types_kv::Response::none(),
        follower2.follower_read(&"missing".to_string()).await??
    );

    Ok(())
}
