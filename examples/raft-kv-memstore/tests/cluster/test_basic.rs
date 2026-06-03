use std::time::Duration;

use openraft::type_config::TypeConfigExt;
use raft_kv_memstore::TypeConfig;

use crate::util;

/// Distinct port range so this test never collides with the others run in parallel.
const PORT_BASE: u16 = 21000;

/// The minimal example: form a cluster, write a key, read it back.
#[test]
fn test_basic() {
    TypeConfig::run(test_basic_inner()).unwrap();
}

async fn test_basic_inner() -> anyhow::Result<()> {
    let client = util::bootstrap(PORT_BASE).await?;

    println!("=== write foo=bar");
    client
        .write(&types_kv::Request::Set {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .await??;

    TypeConfig::sleep(Duration::from_millis(500)).await;

    println!("=== read foo");
    let got = client.read(&"foo".to_string()).await?;
    assert_eq!("bar", got);

    Ok(())
}
