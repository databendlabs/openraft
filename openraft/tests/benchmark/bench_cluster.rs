use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::time::Instant;

use maplit::btreeset;
use openraft::Config;
use tokio::runtime::Builder;

use crate::fixtures::RaftRouter;

struct BenchConfig {
    pub worker_threads: usize,
    pub n_operations: usize,
    pub members: BTreeSet<u64>,
}

impl Display for BenchConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "worker: {}, n: {}, raft_members: {:?}",
            self.worker_threads, self.n_operations, self.members
        )
    }
}

#[test]
#[ignore]
fn bench_cluster_of_1() -> anyhow::Result<()> {
    bench_with_config(&BenchConfig {
        worker_threads: 8,
        n_operations: 100_000,
        members: btreeset! {0},
    })?;
    Ok(())
}

#[test]
#[ignore]
fn bench_cluster_of_3() -> anyhow::Result<()> {
    bench_with_config(&BenchConfig {
        worker_threads: 8,
        n_operations: 100_000,
        members: btreeset! {0,1,2},
    })?;
    Ok(())
}

#[test]
#[ignore]
fn bench_cluster_of_5() -> anyhow::Result<()> {
    bench_with_config(&BenchConfig {
        worker_threads: 8,
        n_operations: 100_000,
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
    let output = rt.block_on(do_bench(bench_config))?;
    Ok(output)
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
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    let _log_index = router.new_nodes_from_single(bench_config.members.clone(), btreeset! {}).await?;

    let n = bench_config.n_operations;

    let now = Instant::now();

    for i in 0..n {
        router.client_request(0, "foo", i as u64).await
    }

    let elapsed = now.elapsed();

    println!(
        "{}: time: {:?}, ns/op: {}, op/ms: {}",
        bench_config,
        elapsed,
        elapsed.as_nanos() / (n as u128),
        (n as u128) / elapsed.as_millis(),
    );

    Ok(())
}
