# Membership example

Runs an in-process OpenRaft cluster with one of six memberships: the default
majority `Membership` of `openraft`, or one of the five memberships of
[`openraft-membership`](../../membership/). Each run initializes the cluster,
changes its membership, isolates some nodes, heals them, and checks every
response and every replicated value along the way.

## Usage

```shell
cargo run --manifest-path examples/membership/Cargo.toml -- --policy weighted
```

`--policy` selects the membership and its edit type:

| `--policy`     | Membership               | Edit                 |
|----------------|--------------------------|----------------------|
| `majority`     | `Membership`             | `ChangeMembers`      |
| `threshold`    | `ThresholdMembership`    | `ThresholdChange`    |
| `weighted`     | `WeightedMembership`     | `WeightedChange`     |
| `hierarchical` | `HierarchicalMembership` | `HierarchicalChange` |
| `grid`         | `GridMembership`         | `GridChange`         |
| `explicit`     | `ExplicitMembership`     | `ExplicitChange`     |

A run logs each phase and ends with `every check passed`, or exits with an
error that describes the state of every node. `RUST_LOG` replaces the default
log filter, `membership=info`. `cargo test` runs every scenario through the same
function as the CLI.

## What a run does

Each scenario in `src/scenarios/` declares a type config with its membership
and edit types, and runs these phases:

1. Log the types, the initial layout, and the read and write quorum rules.
2. Initialize node 1 with an explicit membership through `Raft::initialize()`,
   and wait until node 1 leads and has applied the blank entry of its term.
3. Write a value, and check the complete response and the value on every
   member, including its version.
4. Change the membership with `Raft::change_membership(edit, false)`, using the
   native edit type, and wait until every member applies the uniform result.
5. Write again, and wait until every member applies it.
6. Isolate some nodes. Check the available nodes with `read_quorum_set()` and
   `write_quorum_set()`, then check the write that the cluster commits, or that
   it holds back.
7. Heal the isolation, wake the leader's replication, and check that every
   member applies every write.
8. Shut down every node.

A read quorum elects a leader and a write quorum commits an entry. A newly
elected leader also needs a write quorum: it completes no write or
linearizable read before it commits the blank entry of its term.

`src/cluster.rs` holds the shared `Cluster`. It creates every node with empty
[`log-mem`](../log-mem/) and [`sm-mem`](../sm-mem/) stores, connects them with
the in-process transport of [`network-v2-mem`](../network-v2-mem/), and waits on
metrics. Ticks, heartbeats and elections are off, so node 1 stays the leader in
every phase.

## Scenarios

| `--policy` | Initial configuration                   | Edit                                     | Available during isolation | Result                                         |
|------------|-----------------------------------------|------------------------------------------|----------------------------|------------------------------------------------|
| `majority` | voters `{1,2,3}`; node 4 joins later    | add learner 4, then `AddVoterIds({4})`   | `{1,2,3}`                  | 3 of 4 voters are a majority; writes continue  |
