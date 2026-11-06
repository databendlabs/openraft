# Membership example

Runs an in-process OpenRaft cluster with one of six memberships: the default
majority `Membership` of `openraft`, or one of the five memberships of
[`openraft-membership`](../../membership/).

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

`RUST_LOG` replaces the default log filter, `membership=info`.
