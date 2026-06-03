use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::sync::Once;
use std::thread;
use std::time::Duration;

use app_http::AddLearnerRequest;
use app_http::Client;
use maplit::btreeset;
use openraft::async_runtime::AsyncRuntime;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::AsyncRuntimeOf;
use raft_kv_memstore::TypeConfig;
use raft_kv_memstore::start_example_raft_node;
use tracing_subscriber::EnvFilter;

/// Install a panic hook and tracing subscriber, exactly once per test process.
///
/// `cargo test` runs every `#[test]` in one process on parallel threads, and the
/// global tracing subscriber may only be set once. `Once` keeps a second test
/// from panicking on a double init.
pub fn init_observability() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        std::panic::set_hook(Box::new(log_panic));
        let _ = tracing_subscriber::fmt()
            .with_target(true)
            .with_thread_ids(true)
            .with_level(true)
            .with_ansi(false)
            .with_env_filter(EnvFilter::from_default_env())
            .try_init();
    });
}

fn log_panic(panic: &PanicHookInfo) {
    let backtrace = format!("{:?}", Backtrace::force_capture());

    eprintln!("{}", panic);

    if let Some(location) = panic.location() {
        tracing::error!(
            message = %panic,
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
        eprintln!("{}:{}:{}", location.file(), location.line(), location.column());
    } else {
        tracing::error!(message = %panic, backtrace = %backtrace);
    }

    eprintln!("{}", backtrace);
}

/// The client only knows node ids, so each test maps ids to addresses itself.
///
/// Every test file passes a distinct `base` so their clusters never share a
/// listening port when `cargo test` runs them in parallel.
pub fn api_addr(base: u16, node_id: u64) -> String {
    format!("127.0.0.1:{}", base + node_id as u16)
}

pub fn raft_addr(base: u16, node_id: u64) -> String {
    format!("127.0.0.1:{}", base + 1000 + node_id as u16)
}

/// Start three example raft nodes, each on its own thread and runtime.
pub fn spawn_nodes(base: u16) {
    init_observability();

    for id in 1..=3u64 {
        thread::spawn(move || {
            let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
            let _ = rt.block_on(start_example_raft_node(id, api_addr(base, id), raft_addr(base, id)));
        });
    }
}

/// Start a 3-node cluster and return a client to the leader (node 1).
///
/// Nodes 2 and 3 join as learners and are promoted to voters, so the returned
/// cluster is `{1, 2, 3}` and fully functional. Use this from tests that need a
/// running cluster but are not themselves testing how it is formed.
pub async fn bootstrap(base: u16) -> anyhow::Result<Client<TypeConfig>> {
    spawn_nodes(base);
    TypeConfig::sleep(Duration::from_millis(1_000)).await;

    let leader = Client::<TypeConfig>::new(1, api_addr(base, 1));
    leader.init().await??;

    loop {
        let metrics = leader.metrics().await?;
        if metrics.current_leader == Some(1) {
            break;
        }
        TypeConfig::sleep(Duration::from_millis(200)).await;
    }

    leader
        .add_learner(&AddLearnerRequest {
            node_id: 2,
            api_addr: api_addr(base, 2),
            raft_addr: raft_addr(base, 2),
        })
        .await??;
    leader
        .add_learner(&AddLearnerRequest {
            node_id: 3,
            api_addr: api_addr(base, 3),
            raft_addr: raft_addr(base, 3),
        })
        .await??;

    leader.change_membership(&btreeset! {1, 2, 3}).await??;

    Ok(leader)
}
