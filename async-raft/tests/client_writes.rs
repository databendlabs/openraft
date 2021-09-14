use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::State;
use fixtures::RaftRouter;
use futures::prelude::*;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Client write tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - write a lot of data to it.
/// - assert that the cluster stayed stable and has all of the expected data.
///
/// RUST_LOG=async_raft,memstore,client_writes=trace cargo test -p async-raft --test client_writes
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_writes() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut want = 0;

    // Assert all nodes are in non-voter state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], want, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::NonVoter, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want += 1;

    router.wait_for_log(&btreeset![0, 1, 2], want, None, "leader init log").await?;
    router.wait_for_state(&btreeset![0], State::Leader, None, "init").await?;

    router.assert_stable_cluster(Some(1), Some(want)).await;

    // Write a bunch of data and assert that the cluster stayes stable.
    let leader = router.leader().await.expect("leader not found");
    let mut clients = futures::stream::FuturesUnordered::new();
    clients.push(router.client_request_many(leader, "0", 1000));
    clients.push(router.client_request_many(leader, "1", 1000));
    clients.push(router.client_request_many(leader, "2", 1000));
    clients.push(router.client_request_many(leader, "3", 1000));
    clients.push(router.client_request_many(leader, "4", 1000));
    clients.push(router.client_request_many(leader, "5", 1000));
    while clients.next().await.is_some() {}

    want = 6001;
    router.wait_for_log(&btreeset![0, 1, 2], want, None, "sync logs").await?;

    router.assert_stable_cluster(Some(1), Some(want)).await; // The extra 1 is from the leader's initial commit entry.

    // TODO(xp): flaky test on CI: want voted_for to be Some(0) but is None.
    //           maybe a heavy load delayed heartbeat thus a node start to elect itself. since we have changed follwoer
    //           election timeout to 2 seconds.

    // 17:     0x56062bbec5e1 -
    // client_writes::client_writes::{{closure}}::h3eef34d4ff194d1c at /home/runner/work/async-raft/async-raft/
    // async-raft/tests/client_writes.rs:68:5 18:     0x56062bbff6b9 - <core::future::from_generator::GenFuture<T>
    // as core::future::future::Future>::poll::h8528880ed4984b5f at /rustc/657bc01888e6297257655585f9c475a0801db6d2/
    // library/core/src/future/mod.rs:80:19 19:     0x56062bc3abf0 -
    // tokio::park::thread::CachedParkThread::block_on::{{closure}}::h23f5d1216c312664 at /home/runner/.cargo/
    // registry/src/github.com-1ecc6299db9ec823/tokio-1.11.0/src/park/thread.rs:263:54 20:     0x56062ba82a02 -
    // tokio::coop::with_budget::{{closure}}::h578cdc75da828f49 at /home/runner/.cargo/registry/src/github.
    // com-1ecc6299db9ec823/tokio-1.11.0/src/coop.rs:106:9 21:     0x56062bbadcc3 -
    // std::thread::local::LocalKey<T>::try_with::h606128d4eea7b416
    // at /rustc/657bc01888e6297257655585f9c475a0801db6d2/library/std/src/thread/local.rs:400:16
    // 22:     0x56062bbad67d - std::thread::local::LocalKey<T>::with::h774dc94ef7a26b1e
    // at /rustc/657bc01888e6297257655585f9c475a0801db6d2/library/std/src/thread/local.rs:376:9
    // 23:     0x56062bc3a531 - tokio::coop::with_budget::h0f24acfd1b70670f
    // at /home/runner/.cargo/registry/src/github.com-1ecc6299db9ec823/tokio-1.11.0/src/coop.rs:99:5
    // 24:     0x56062bc3a531 - tokio::coop::budget::h0db654af8d5d8547
    // at /home/runner/.cargo/registry/src/github.com-1ecc6299db9ec823/tokio-1.11.0/src/coop.rs:76:5
    // 25:     0x56062bc3a531 - tokio::park::thread::CachedParkThread::block_on::h502d780296c510ac
    // at /home/runner/.cargo/registry/src/github.com-1ecc6299db9ec823/tokio-1.11.0/src/park/thread.rs:263:31
    // 26:     0x56062bb48b6e - tokio::runtime::enter::Enter::block_on::h6bde65108754726b
    // at /home/runner/.cargo/registry/src/github.com-1ecc6299db9ec823/tokio-1.11.0/src/runtime/enter.rs:151:13
    // 27:     0x56062badf323 - tokio::runtime::thread_pool::ThreadPool::block_on::ha1bc65b6c611f8a0
    // at /home/runner/.cargo/registry/src/github.com-1ecc6299db9ec823/tokio-1.11.0/src/runtime/thread_pool/mod.rs:72:9
    // 28:     0x56062bacaf0b - tokio::runtime::Runtime::block_on::h6d77b2fd845d5815
    // at /home/runner/.cargo/registry/src/github.com-1ecc6299db9ec823/tokio-1.11.0/src/runtime/mod.rs:459:43
    // 29:     0x56062bacf0b4 - client_writes::client_writes::hbfbb9022948a574e
    // at /home/runner/work/async-raft/async-raft/async-raft/tests/client_writes.rs:81:5
    // 30:     0x56062bbe8e7e - client_writes::client_writes::{{closure}}::hfba876b294dbbd4d
    // at /home/runner/work/async-raft/async-raft/async-raft/tests/client_writes.rs:25:7

    router
        .assert_storage_state(
            1,
            want,
            Some(0),
            LogId { term: 1, index: want },
            Some(((5000..5100).into(), 1, MembershipConfig {
                members: btreeset![0, 1, 2],
                members_after_consensus: None,
            })),
        )
        .await;

    Ok(())
}
