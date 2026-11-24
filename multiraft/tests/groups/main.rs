//! Tests of Raft groups whose members share one process and one [`GroupRouter`].
//!
//! [`GroupRouter`]: openraft_multi::GroupRouter

mod group_network;
mod local_members;
mod t50_blocked_state_machine;

use std::sync::Arc;

use openraft_memstore::MemStateMachine;

type C = openraft_memstore::TypeConfig;
type Raft = openraft::Raft<C, Arc<MemStateMachine>>;

/// Send the logs to the test harness, which prints them only when a test fails. `RUST_LOG` selects
/// the logs; without it, only errors are logged.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::from_default_env();
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter).with_test_writer();
    // Another test of this binary may have installed it.
    subscriber.try_init().ok();
}
