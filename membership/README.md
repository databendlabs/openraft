# openraft-membership

Ready-to-use membership implementations for [Openraft](https://github.com/databendlabs/openraft).

A membership sets the read quorums that elect a leader and the write quorums
that commit an entry. Openraft's default `Membership` uses a majority for both.
This crate provides other quorum rules, and each one makes every read quorum
share a voter with every write quorum:

| Membership | Change | A config sets |
|---|---|---|
| `ThresholdMembership` | `ThresholdChange` | the voters and a write size; the read size is derived |
| `WeightedMembership` | `WeightedChange` | a weight for each voter and a write weight |
| `HierarchicalMembership` | `HierarchicalChange` | a tree of voter groups, such as zones, with a write size for each |
| `GridMembership` | `GridChange` | a rectangular grid: a full row elects, a voter of each row commits |
| `ExplicitMembership` | `ExplicitChange` | the minimal read quorums and the minimal write quorums |

The weighted, hierarchical, grid and explicit memberships can also give a node
a role other than its default, such as a voter that votes and stores logs but
never starts an election.

## Usage

Select a membership and its change type in the type config:

```rust,ignore
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Membership = openraft_membership::WeightedMembership<u64, BasicNode>,
        MembershipEdit = openraft_membership::WeightedChange<Self::NodeId, Self::Node>,
);
```

Then pass a membership to `Raft::initialize()`, and a change to
`Raft::change_membership()`:

```rust,ignore
use std::collections::BTreeMap;

use openraft_membership::{WeightedChange, WeightedMembership};

// Voter 1 weighs 2 of the total 5; a commit needs weight 3.
let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
let membership = WeightedMembership::new(weights, 3, nodes)?;
raft.initialize(membership).await?;

// Give every voter weight 1, keeping write weight 3.
let change = WeightedChange::weights(BTreeMap::from([(1, 1), (2, 1), (3, 1), (4, 1)]));
raft.change_membership(change, false).await?;
```

## Example

[`raft-kv-memstore-weighted-membership`](https://github.com/databendlabs/openraft/tree/main/examples/raft-kv-memstore-weighted-membership)
is the HTTP key-value store example with a `WeightedMembership`. Its `/init`
and `/change-membership` routes take the weight of each voter, and its test
stops two voters and checks that the others, whose weights reach the write
weight, keep committing.

## Features

- `single-threaded`: enables `openraft/single-threaded`.

The memberships implement serde whenever `openraft/serde` is on, which is
when `RaftMembership` requires it, so this crate has no `serde` feature.

## Documentation

The [`docs`](https://docs.rs/openraft-membership/latest/openraft_membership/docs/index.html)
module tells which membership fits which cluster, and holds a guide for each.
