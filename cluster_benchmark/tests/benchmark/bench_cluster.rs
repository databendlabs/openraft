use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use maplit::btreeset;
use openraft::Config;
use tokio::runtime::Builder;

use crate::network::Router;
use crate::store::ClientRequest;

struct BenchConfig {
    /// Worker threads for both client and server tasks
    pub worker_threads: usize,
    pub n_operations: u64,
    pub n_client: u64,
    pub members: BTreeSet<u64>,
}

impl Display for BenchConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "workers: {}, clients: {}, n: {}, raft_members: {:?}",
            self.worker_threads, self.n_client, self.n_operations, self.members
        )
    }
}

#[test]
#[ignore]
fn bench_cluster_of_1() -> anyhow::Result<()> {
    bench_with_config(&BenchConfig {
        worker_threads: 32,
        n_operations: 100_000,
        n_client: 256,
        members: btreeset! {0},
    })?;
    Ok(())
}

#[test]
#[ignore]
fn bench_cluster_of_3() -> anyhow::Result<()> {
    bench_with_config(&BenchConfig {
        worker_threads: 32,
        n_operations: 100_000,
        n_client: 256,
        members: btreeset! {0,1,2},
    })?;
    Ok(())
}

#[test]
#[ignore]
fn bench_cluster_of_5() -> anyhow::Result<()> {
    bench_with_config(&BenchConfig {
        worker_threads: 32,
        n_operations: 100_000,
        n_client: 256,
        members: btreeset! {0,1,2,3,4},
    })?;
    Ok(())
}

fn bench_with_config(bench_config: &BenchConfig) -> anyhow::Result<()> {
    let rt = Builder::new_multi_thread()
        .worker_threads(bench_config.worker_threads)
        .enable_all()
        .thread_name("bench-cluster")
        .thread_stack_size(3 * 1024 * 1024)
        .build()?;

    // Run client_write benchmark
    rt.block_on(do_bench(bench_config))
}

/// Benchmark client_write.
///
/// Cluster config:
/// - Log: in-memory BTree
/// - StateMachine: in-memory BTree
async fn do_bench(bench_config: &BenchConfig) -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 200,
            election_timeout_max: 2000,
            purge_batch_size: 1024,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = Router::new();
    router.new_cluster(config.clone(), bench_config.members.clone()).await?;

    let n = bench_config.n_operations;
    let total = n * bench_config.n_client;

    let leader = router.get_raft(0);
    let mut handles = Vec::new();

    // Benchmark start
    let now = Instant::now();
    for _nc in 0..bench_config.n_client {
        let l = leader.clone();
        let h = tokio::spawn(async move {
            for _i in 0..n {
                l.client_write(ClientRequest {})
                    .await
                    .map_err(|e| {
                        eprintln!("client_write error: {:?}", e);
                        e
                    })
                    .unwrap();
            }
        });

        handles.push(h)
    }

    leader.wait(timeout()).applied_index_at_least(Some(total), "commit all written logs").await?;

    let elapsed = now.elapsed();

    for h in handles {
        h.await?;
    }

    println!(
        "{}: time: {:?}, ns/op: {}, op/ms: {}",
        bench_config,
        elapsed,
        elapsed.as_nanos() / (total as u128),
        (total as u128) / elapsed.as_millis(),
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(50_000))
}
