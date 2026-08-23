# Openraft integration tests

The integration tests for Openraft are stored in this directory and rely on
`memstore` with `serde` enabled.
Certain tests in Openraft require the `serde` feature to be disabled.
To avoid enabling `serde` for all tests in Openraft, we must relocate the
integration tests to a separate crate.


## Case naming convention

A test file name starts with `t[\d\d]_`, where `\d\d` is the test case number indicating priority.

- `t00`: not used.
- `t10`: basic behaviors.
- `t20`: life cycle test cases. 
- `t30`: special cases for an API. 
- `t40`: not used. 
- `t50`: environment depended behaviors.  
- `t60`: config related behaviors. 
- `t70`: not used.
- `t80`: not used.
- `t90`: issue fixes. 


## Adding a test case

Every directory here, such as `metrics/`, builds one test binary. Put the new
file in the directory that owns the behavior under test, then declare it in
that directory's `main.rs`:

```rust
mod t10_my_case;
```

Keep the `mod` list in file-name order. The numeric prefix describes the
behavior's logical layer. Lower numbers cover basic behavior, while higher
numbers cover higher-level behavior built from those basics.

`custom_type_config_test.rs` and `public_api_test.rs` sit at the top level
instead of in a directory. Each is a single-file binary that only has to
compile, so neither needs a directory of its own.


## Test case skeleton

A test case is one async function that carries both attributes, returns
`Result<()>`, and builds its own `Config` and `RaftRouter`:

```rust
/// One line stating what this case proves.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn my_case() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up a 3-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write one log");
    {
        router.client_request(0, "foo", 1).await?;
        log_index += 1;

        router.wait(&0, timeout()).applied_index(Some(log_index), "write one log").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
```

Turn off `enable_tick`, `enable_heartbeat`, or `enable_elect` in `Config` when
a background timer would disturb what the case observes. Set the flag before
creating the router, and use `Raft::runtime_config()` only when the case must
flip it in the middle of a run.

Each file defines its own finite `timeout()` at the bottom and uses it for
every `wait()` that expects progress. One second suits most cases.

`ut_harness` installs the tracing subscriber and the panic hook, and runs the
test body on the async runtime. It writes every log line to `tests/_log/`,
which is the first place to look when a case fails.

A case that touches no cluster, such as the serialization checks in
`snapshot_streaming/t10_wire_compat_09.rs`, is a plain `#[test]` function
with neither attribute. It needs no async runtime, so it needs no harness.


## Set up the cluster with `new_cluster`

`RaftRouter::new_cluster(voter_ids, learner_ids)` creates every node,
initializes the cluster, turns the remaining voters into voters, adds the
learners, waits for all of them to catch up, and returns the last log index:

```rust
let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
```

Use it in every case that only needs a running cluster. Call
`Raft::initialize()` by hand only when the case observes initialization
itself, for example an uninitialized node or an `initialize()` error.

A case that drives one Raft API directly, such as
`append_entries/t11_append_conflicts.rs` calling `Raft::append_entries()`,
creates its nodes with `new_raft_node()` and forms no cluster at all.

A hand-written setup must wait for the leader to apply log index 1 before it
calls `add_learner()` or `change_membership()`:

```rust
n0.initialize(btreeset! {0}).await?;
router.wait(&0, timeout()).applied_index(Some(1), "init").await?;
```

`Raft::initialize()` returns once the membership entry at index 0 is flushed,
not once it is committed. A node also reports `ServerState::Leader` before it
commits its own blank log at index 1. A membership call that arrives in that
window is rejected with `ChangeMembershipError::InProgress`, which surfaces as
a test that fails only now and then on CI.


## Track the expected log index

Keep one `log_index` variable that mirrors the last log index the cluster
should hold, and update it right after each write:

- cluster init: 2 logs, the membership entry at index 0 and the leader blank
  log at index 1.
- `add_learner()`: 1 log.
- `change_membership()`: 2 logs, the joint config and then the uniform config.
- `client_request()`: 1 log. `client_request_many()` returns how many it wrote.

Assert progress with a wait, never with a sleep:

```rust
router.wait(&node_id, timeout()).applied_index(Some(log_index), "write one log").await?;
```

`Wait` also offers `committed_index()`, `state()`, `vote()`, `members()`,
`voter_ids()`, `snapshot()`, `purged()`, and `current_leader()`. Passing
`None` creates a 100-year timeout. Use it only when an explicit finite
`TypeConfig::timeout()` bounds the whole wait.

Sleep only to show that something does not happen. A `TypeConfig::sleep()`
followed by an assertion that the term did not grow, or that the node is
still the leader, is the one correct use. Await everything a case expects to
happen with `wait()` instead, because a sleep long enough to be safe on a
loaded CI runner also slows down every green run.


## Fault injection

The router owns the simulated network, so a case shapes failures through it:

- `set_network_error(id, bool)` and `set_unreachable(id, bool)` cut a node off.
- `set_rpc_failure(id, direction, error_type)` fails one direction of one node.
- `set_rpc_pre_hook()` and `set_rpc_post_hook()` block or inspect one RPC type.
- `network_send_delay(ms)` delays every send.
- `get_rpc_count()` reports how many RPCs of each type were sent.

CI runs the whole suite a second time with `OPENRAFT_NETWORK_SEND_DELAY=30`,
which delays every RPC by 30 ms. A case must therefore never assume an RPC
completes quickly.
