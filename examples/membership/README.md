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

Each scenario checks the available nodes against the quorums of the membership
after the edit. Node 1 leads in every phase and stays available.

| `--policy`     | Initial configuration                                                      | Edit                                       | Available during isolation | Result                                                                                          |
|----------------|----------------------------------------------------------------------------|--------------------------------------------|----------------------------|-------------------------------------------------------------------------------------------------|
| `majority`     | voters `{1,2,3}`; node 4 starts outside                                    | add learner 4, then `AddVoterIds({4})`     | `{1,2,3}`                  | 3 of the 4 voters are a majority: writes continue                                               |
| `threshold`    | voters `{1,2,3,4,5}`, write size 3                                         | `write_size(2)`: the read size becomes 4   | `{1,2}`                    | writes continue with 2 copies; no read quorum is left for an election                           |
| `weighted`     | weights `{1:2, 2:1, 3:1, 4:1}`, write weight 3                             | `weights({1:1, 2:2, 3:1, 4:1})`            | `{1,2}`                    | weight 3 reaches the read weight and the write weight: writes continue                          |
| `hierarchical` | zones `{1,2,3}`, `{4,5,6}`, `{7,8,9}`; a write needs 2 voters in each zone | `groups(..)`: a write needs 2 of the zones | `{1,2,3,4,5,6}`            | 2 voters in each of 2 zones are a read quorum and a write quorum: writes continue               |
| `grid`         | rows `[[1,2,3],[4,5,6],[7,8,9]]`                                           | `grid([[1,4,7],[2,5,8],[3,6,9]])`          | `{1,5,9}`                  | a voter of each new row commits; no complete row is left for an election                        |
| `explicit`     | reads: any 2 of `{1,2,3}`; writes: each such pair plus backup voter 9      | `read_quorums([{1,2}])`                    | `{1,2,3}`                  | the read quorum `{1,2}` is available, but a write waits for voter 9 and completes after healing |

The threshold, weighted, hierarchical and grid edits would let an old read
quorum miss a new write quorum, or an old write quorum miss a new read quorum.
So `change_membership()` first appends a joint config, whose quorums are quorums
of both configs, and then the uniform config. The majority and explicit edits
keep both pairs intersecting, so OpenRaft appends the uniform config alone.
