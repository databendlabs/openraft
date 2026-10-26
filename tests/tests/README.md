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
- `t40`: behaviors under a provided membership type other than the default `Membership`.
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
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a 3-node cluster");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- write one log");
    {
        let response = router.client_request(0, "foo", 1).await?;
        let index = response.log_id.index();
        router.wait_applied_at_least([0, 1, 2], index, "write applied").await?;
    }

    Ok(())
}
```

Turn off `enable_tick`, `enable_heartbeat`, or `enable_elect` in `Config` when
a background timer would disturb what the case observes. Set the flag before
creating the router, and use `Raft::runtime_config()` only when the case must
flip it in the middle of a run.

`router.wait(&id)` uses the shared five-second deadline. Use
`router.wait_with_timeout(&id, duration)` for a specific observation window.
Direct Raft API calls can use `fixtures::timeout()`.

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
let log_index = router.new_cluster([0, 1, 2], [3]).await?;
```

The constructor validates `Config` and shares it internally. `new_cluster()`
accepts arrays or other iterators. It checks that voter 0 is present, voters and
learners are disjoint, and all requested IDs are unused before starting nodes.
`wait_applied_at_least()` gives the whole node group one five-second deadline
and accepts progress beyond the requested index. Keep exact-index waits when
advancing beyond that boundary would indicate a bug.

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
router.wait(&0).applied_index(Some(1), "init").await?;
```

`Raft::initialize()` returns once the membership entry at index 0 is flushed,
not once it is committed. A node also reports `ServerState::Leader` before it
commits its own blank log at index 1. A membership call that arrives in that
window is rejected with `ChangeMembershipError::UncommittedLeaderLog`, which
surfaces as a test that fails only now and then on CI.


## Track the expected log index

Keep one `log_index` variable that mirrors the last log index the cluster
should hold, and update it right after each write:

- cluster init: 2 logs, the membership entry at index 0 and the leader blank
  log at index 1.
- `add_learner()`: 1 log.
- `change_membership()`: 1 log, the uniform config, when every quorum of the old
  voter set intersects every quorum of the new one, such as when it adds or
  removes one voter. Otherwise 2 logs, the joint config and then the uniform
  config.
- `client_request()`: one strict write to the requested node, returning the full
  `ClientWriteResponse`. `write()` accepts an explicit `ClientRequest`.
- `write_following_leader()`: follows up to three redirects when a workload
  explicitly needs leader discovery.
- `client_request_many()`: stops on the first error and returns `WriteSummary`
  with the acknowledged count and final log ID (`None` for an empty workload).

Use response log IDs for ordinary synchronization. Keep independent index
arithmetic when the test verifies entry counts or ordering. Consume concurrent
workload results with `try_next().await?` so write errors reach the test.

Assert progress with a wait, never with a sleep:

```rust
router.wait(&node_id).applied_index(Some(log_index), "write one log").await?;
```

`Wait` also offers `committed_index()`, `state()`, `vote()`, `members()`,
`voter_ids()`, `snapshot()`, `purged()`, and `current_leader()`. Passing
`None` to `Raft::wait()` disables its deadline. Use it only inside a finite
outer timeout. `ut_harness` bounds each test to 120 seconds; set
`OPENRAFT_TEST_TIMEOUT_SECS` to a positive integer to change that limit.
Use `fixtures::expect_timeout(result)?` for negative metric observations, so
node shutdown fails the case instead of satisfying the observation.

Sleep only to show that something does not happen. A `TypeConfig::sleep()`
followed by an assertion that the term did not grow, or that the node is
still the leader, is the one correct use. Await everything a case expects to
happen with `wait()` instead, because a sleep long enough to be safe on a
loaded CI runner also slows down every green run.


## Fault injection

The router sends full snapshots directly by default. Select
`Builder::chunked_snapshots()` to exercise the legacy chunking adapter.
`router.snapshot_chunks()` returns the complete wire chunk sequence for assertions.

The router owns the simulated network, so a case shapes failures through it:

- `set_network_error(id, bool)` and `set_unreachable(id, bool)` cut a node off.
- `set_rpc_failure(id, direction, error_type)` fails one direction of one node.
- `set_rpc_pre_hook()` and `set_rpc_post_hook()` block or inspect one RPC type.
- `network_send_delay(ms)` delays every send.
- `get_rpc_count()` reports how many RPCs of each type were sent.

Use `RpcRule` for one RPC kind on a directed link:

```rust
let rule = RpcRule::new(0, 1, RpcKind::AppendEntries);
let blocked = router.add_rpc_rule(rule);
// Run the isolated phase.
router.remove_rpc_rule(blocked);
```

Rules apply in insertion order, before custom pre-hooks. Removing one rule
leaves the other rules active. `RpcRule::matching()` restricts a rule by its
request contents. `get_rpc_kind_count()` distinguishes Vote from PreVote and
snapshot chunks from full snapshot transfers. Post-hooks observe the final
transport response, including quota-induced `PartialSuccess`.

CI runs the whole suite a second time with `OPENRAFT_NETWORK_SEND_DELAY=30`,
which delays every RPC by 30 ms. A case must therefore never assume an RPC
completes quickly.
