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
    let first = client
        .write(&types_kv::Request::Set {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .await??;
    let first_version = first.data.value.as_ref().unwrap().version;

    println!("=== write foo=bar again");
    let second = client
        .write(&types_kv::Request::Set {
            key: "foo".to_string(),
            value: "bar".to_string(),
        })
        .await??;
    let second_version = second.data.value.as_ref().unwrap().version;
    assert!(second_version > first_version);

    TypeConfig::sleep(Duration::from_millis(500)).await;

    println!("=== read foo");
    let got = client.read(&"foo".to_string()).await?;
    assert_eq!(second.data, got);

    println!("=== compare and set foo=bar to foo=baz");
    let cas = client.write(&types_kv::Request::compare_and_set("foo", second_version, "baz")).await??;
    let cas_value = cas.data.value.as_ref().unwrap();
    assert_eq!("baz", cas_value.value);
    assert!(cas_value.version > second_version);

    println!("=== reject a stale compare and set");
    let stale = client.write(&types_kv::Request::compare_and_set("foo", second_version, "stale")).await??;
    assert_eq!(types_kv::Response::none(), stale.data);
    assert_eq!(cas.data, client.read(&"foo".to_string()).await?);

    println!("=== reject a compare and set for a missing key");
    let missing = client.write(&types_kv::Request::compare_and_set("missing", 0, "value")).await??;
    assert_eq!(types_kv::Response::none(), missing.data);
    assert_eq!(types_kv::Response::none(), client.read(&"missing".to_string()).await?);

    println!("=== reject a compare and set after ABA");
    let original = client.write(&types_kv::Request::set("aba", "A")).await??;
    let original_version = original.data.value.as_ref().unwrap().version;
    client.write(&types_kv::Request::set("aba", "B")).await??;
    let restored = client.write(&types_kv::Request::set("aba", "A")).await??;
    let restored_version = restored.data.value.as_ref().unwrap().version;
    assert!(restored_version > original_version);

    let stale = client.write(&types_kv::Request::compare_and_set("aba", original_version, "C")).await??;
    assert_eq!(types_kv::Response::none(), stale.data);
    assert_eq!(restored.data, client.read(&"aba".to_string()).await?);

    Ok(())
}
