//! End-to-end test of the kvstore example: spawn real `kvstore` processes,
//! form a three-node cluster, and drive it purely over HTTP - the same
//! commands the example's doc header shows, minus the terminals.

use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use serde_json::Value;
use serde_json::json;

/// Upper bound for every wait in this test
const WAIT: Duration = Duration::from_secs(30);

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A kvstore child process, killed when the test ends, pass or fail.
struct Node {
    child: Child,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The example binary, which `cargo test` builds alongside the tests.
fn kvstore_bin() -> PathBuf {
    // current_exe is target/debug/deps/kvstore_example-<hash>
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    path.pop();
    path.push("examples");
    path.push(format!("kvstore{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "kvstore example not built at {}; run a full `cargo test -p ezraft`",
        path.display()
    );
    path
}

/// Grab a free port; the listener is dropped so the child process can bind it.
fn free_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// Spawn one kvstore node; its `./data/<addr>` dir and log land in `work_dir`.
fn spawn_node(work_dir: &Path, addr: &str, seed: Option<&str>) -> io::Result<Node> {
    let log = File::create(work_dir.join(format!("node-{}.log", addr.replace(':', "-"))))?;
    let mut cmd = Command::new(kvstore_bin());
    cmd.current_dir(work_dir)
        .args(["--addr", addr])
        .stdout(log.try_clone()?)
        .stderr(log)
        .stdin(Stdio::null());
    if let Some(seed) = seed {
        cmd.args(["--seed", seed]);
    }
    Ok(Node { child: cmd.spawn()? })
}

/// POST a request to a node's write API, retrying while the node's HTTP
/// server is still binding or the cluster has no usable leader yet.
async fn write(addr: &str, req: Value) -> io::Result<Value> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + WAIT;
    loop {
        let res = client.post(format!("http://{}/api/write", addr)).json(&req).send().await;
        match res {
            Ok(resp) if resp.status().is_success() => return resp.json().await.map_err(io::Error::other),
            _ if Instant::now() < deadline => tokio::time::sleep(POLL_INTERVAL).await,
            Ok(resp) => return Err(io::Error::other(format!("write to {} failed: {}", addr, resp.status()))),
            Err(e) => return Err(io::Error::other(e)),
        }
    }
}

/// Poll a node's direct-read endpoint until `key` serves exactly `expected`,
/// where `None` is the 404 of a key the node does not hold. Getting there
/// proves the state reached that node's own state machine.
async fn wait_for_key(addr: &str, key: &str, expected: Option<&Value>) -> io::Result<()> {
    let deadline = Instant::now() + WAIT;
    let mut last = None;
    loop {
        // An error is the node's HTTP server still binding; retry until the deadline.
        if let Ok(resp) = reqwest::get(format!("http://{}/api/read?key={}", addr, key)).await {
            let value = match resp.status() {
                reqwest::StatusCode::NOT_FOUND => None,
                status if status.is_success() => Some(resp.json().await.map_err(io::Error::other)?),
                status => {
                    return Err(io::Error::other(format!(
                        "read {} from {} failed: {}",
                        key, addr, status
                    )));
                }
            };
            if value.as_ref() == expected {
                return Ok(());
            }
            last = Some(value);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "{} never served {:?} for {}, last seen: {:?}",
                addr, expected, key, last
            )));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn three_node_kvstore_cluster_writes_and_reads() -> io::Result<()> {
    let work_dir = std::env::temp_dir().join(format!("ezraft-kvstore-{}", std::process::id()));
    std::fs::create_dir_all(&work_dir)?;

    let addr_a = free_addr();
    let a = spawn_node(&work_dir, &addr_a, None)?;

    // The first write doubles as the readiness wait: it only succeeds once the
    // founding node serves HTTP and leads.
    assert_eq!(
        json!({"value": null}),
        write(&addr_a, json!({"Set": {"key": "k1", "value": "v1"}})).await?
    );

    let addr_b = free_addr();
    let addr_c = free_addr();
    let b = spawn_node(&work_dir, &addr_b, Some(&addr_a))?;
    let c = spawn_node(&work_dir, &addr_c, Some(&addr_a))?;

    // Replication must deliver the pre-join write to both joiners.
    wait_for_key(&addr_b, "k1", Some(&json!("v1"))).await?;
    wait_for_key(&addr_c, "k1", Some(&json!("v1"))).await?;

    // Writes sent to joiners are forwarded to the leader: a set through one,
    // and a delete through the other returning the removed value.
    assert_eq!(
        json!({"value": null}),
        write(&addr_b, json!({"Set": {"key": "k2", "value": "v2"}})).await?
    );
    assert_eq!(
        json!({"value": "v1"}),
        write(&addr_c, json!({"Delete": {"key": "k1"}})).await?
    );

    // Overwriting a key hands back the value it replaced.
    assert_eq!(
        json!({"value": "v2"}),
        write(&addr_a, json!({"Set": {"key": "k2", "value": "v2b"}})).await?
    );

    // Every node converges on the exact final state: the overwritten key reads
    // back everywhere, and the deleted key is a 404 rather than a null.
    for addr in [&addr_a, &addr_b, &addr_c] {
        wait_for_key(addr, "k2", Some(&json!("v2b"))).await?;
        wait_for_key(addr, "k1", None).await?;
    }

    drop(a);
    drop(b);
    drop(c);
    // Only removed on success; a failure leaves data dirs and logs behind for
    // debugging.
    std::fs::remove_dir_all(&work_dir)?;
    Ok(())
}
