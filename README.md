<div align="center">
    <h1>Openraft</h1>
    <h4>
        Advanced <a href="https://raft.github.io/">Raft</a> in 🦀 Rust using <a href="https://tokio.rs/">Tokio</a>. Please ⭐ on <a href="https://github.com/datafuselabs/openraft">github</a>!
    </h4>


[![Crates.io](https://img.shields.io/crates/v/openraft.svg)](https://crates.io/crates/openraft)
[![docs.rs](https://docs.rs/openraft/badge.svg)](https://docs.rs/openraft)
[![guides](https://img.shields.io/badge/guide-%E2%86%97-brightgreen)](https://docs.rs/openraft/latest/openraft/docs/index.html)
[![Discord Chat](https://img.shields.io/discord/1015845055434588200?logo=discord)](https://discord.gg/ZKw3WG7FQ9)
<br/>
[![CI](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml/badge.svg)](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml)
![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)
![Crates.io](https://img.shields.io/crates/d/openraft.svg)
![Crates.io](https://img.shields.io/crates/dv/openraft.svg)

</div>

🪵🪵🪵 Raft is not yet good enough.
This project intends to improve raft as the next-generation consensus protocol for distributed data storage systems (SQL, NoSQL, KV, Streaming, Graph ... or maybe something more exotic).

Currently, openraft is the consensus engine of meta-service cluster in [databend](https://github.com/datafuselabs/databend).


- 🚀 **Get started**:
    - [Openraft guide](https://docs.rs/openraft/latest/openraft/docs/getting_started/index.html) is the best place to get started,
    - [Openraft docs](https://docs.rs/openraft/latest/openraft/docs/index.html) for more in-depth details,
    - [Openraft FAQ](https://docs.rs/openraft/latest/openraft/docs/faq/index.html) explains some common questions.

- 🙌 **Questions**?
    - Join the [Discord channel](https://discord.gg/ZKw3WG7FQ9),
    - or start a [Discussion](https://github.com/datafuselabs/openraft/discussions/new),
    - or join our wechat group: `drmingdrmer`.

- Openraft is derived from [async-raft](https://docs.rs/crate/async-raft/latest) with several bugs fixed: [Fixed bugs](https://github.com/datafuselabs/openraft/blob/main/derived-from-async-raft.md).


# Status

- The features are almost complete for building an application.
- Performance: Supports 70,000 writes/sec for single writer, and 1,000,000 writes/sec for 256 writers. See: [Performance](#performance)
- Unit test coverage stands at 92%.
- The chaos test has not yet been completed, and further testing is needed to ensure the application's robustness and reliability.


## API status

- **Openraft API is not stable yet**. Before `1.0.0`, an upgrade may contain incompatible changes.
  Check our [change-log](https://github.com/datafuselabs/openraft/blob/main/change-log.md). A commit message starts with a keyword to indicate the modification type of the commit:

  - `DataChange:` on-disk data types changes, which may require manual upgrade.
  - `Change:` if it introduces incompatible changes.
  - `Feature:` if it introduces compatible non-breaking new features.
  - `Fix:` if it just fixes a bug.

## Versions

- **Branch main** has been under active development.
    The main branch is for the [release-0.9](https://github.com/datafuselabs/openraft/tree/release-0.9).

- **Branch [release-0.8](https://github.com/datafuselabs/openraft/tree/release-0.8)**:
  Latest: ( [v0.8.8](https://github.com/datafuselabs/openraft/tree/v0.8.8) | [Change log](https://github.com/datafuselabs/openraft/blob/release-0.8/change-log.md#v088) );
  Upgrade guide: ⬆️  [0.7 to 0.8](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_07_08/index.html), ⬆️  [0.8.3 to 0.8.4](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_083_084/index.html);
  `release-0.8` **Won't** accept new features but only bug fixes.

- **Branch [release-0.7](https://github.com/datafuselabs/openraft/tree/release-0.7)**:
  Latest: ( [v0.7.6](https://github.com/datafuselabs/openraft/tree/v0.7.6) | [Change log](https://github.com/datafuselabs/openraft/blob/release-0.7/change-log.md#v076) );
  Upgrade guide: ⬆️  [0.6 to 0.7](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_06_07/index.html);
  `release-0.7` **Won't** accept new features but only bug fixes.

- **Branch [release-0.6](https://github.com/datafuselabs/openraft/tree/release-0.6)**:
  Latest: ( [v0.6.8](https://github.com/datafuselabs/openraft/tree/v0.6.8) | [Change log](https://github.com/datafuselabs/openraft/blob/release-0.6/change-log.md) );
  `release-0.6` **won't** accept new features but only bug fixes.

# Roadmap

- [x] **2022-10-31** [Extended joint membership](https://docs.rs/openraft/latest/openraft/docs/data/extended_membership/index.html)
- [x] **2023-02-14** Minimize confliction rate when electing;
  See: [Openraft Vote design](https://docs.rs/openraft/latest/openraft/docs/data/vote/index.html);
  Or use standard raft mode with [feature flag `single-term-leader`](https://docs.rs/openraft/latest/openraft/docs/feature_flags/index.html).
- [x] **2023-04-26** Goal performance is 1,000,000 put/sec.
- [ ] Reduce the complexity of vote and pre-vote: [get rid of pre-vote RPC](https://github.com/datafuselabs/openraft/discussions/15);
- [ ] Support flexible quorum, e.g.: [Hierarchical Quorums](https://zookeeper.apache.org/doc/r3.5.9/zookeeperHierarchicalQuorums.html)
- [ ] Consider introducing read-quorum and write-quorum,
  improve efficiency with a cluster with an even number of nodes.


<!--
   - - [ ] Consider to separate log storage and log order storage.
   -   Leader only determines and replicates the index of log entries, not log
   -   payload.
      -->

# Performance

The benchmark is focused on the Openraft framework itself and is run on a
minimized store and network. This is **NOT a real world** application benchmark!!!

| clients | put/s         | ns/op      |
| --:     | --:           | --:        |
| 256     | **1,014,000** |      985   |
|  64     |   **730,000** |    1,369   |
|   1     |      70,000   | **14,273** |


For benchmark detail, go to the [./cluster_benchmark](./cluster_benchmark) folder.

# Features

- **Async and Event-Driven**: Operates based on Raft events without reliance on periodic ticks, optimizing message batching for high throughput.
- **Extensible Storage and Networking**: Customizable via `RaftLogStorage`, `RaftStateMachine` and `RaftNetwork` traits, allowing flexibility in choosing storage and network solutions.
- **Unified Raft API**: Offers a single `Raft` type for creating and interacting with Raft tasks, with a straightforward API.
- **Cluster Formation**: Provides strategies for initial cluster setup as detailed in the [cluster formation guide](https://docs.rs/openraft/latest/openraft/docs/cluster_control/cluster_formation/index.html).
- **Built-In Tracing Instrumentation**: The codebase integrates [tracing](https://docs.rs/tracing/) for logging and distributed tracing, with the option to [set verbosity levels at compile time](https://docs.rs/tracing/latest/tracing/level_filters/index.html).

## Functionality:

- ✅ **Leader election**: by policy or manually([`trigger_elect()`][]).
- ✅ **Non-voter(learner) Role**: refer to [`add_learner()`][].
- ✅ **Log Compaction**(snapshot of state machine): by policy or manully([`trigger_snapshot()`]).
- ✅ **Snapshot replication**.
- ✅ **Dynamic Membership**: using joint membership config change. Refer to [dynamic membership](https://docs.rs/openraft/latest/openraft/docs/cluster_control/dynamic_membership/index.html)
- ⛔️ **Wont support**: Single-step config change.  <!-- TODO: explain why -->
- ✅ Toggle heartbeat: [`enable_heartbeat`][].
- ✅ Toggle election: [`enable_elect`][].
- ✅ Purge log by policy or manually: [`purge_log()`][].

<!--
TODO: 0.9
- ✅ Linearizable read.
-->


# Who use it

- [yuyang0/rrqlite](https://github.com/yuyang0/rrqlite)
- [raymondshe/matchengine-raft](https://github.com/raymondshe/matchengine-raft)

# Contributing

Check out the [CONTRIBUTING.md](https://github.com/datafuselabs/openraft/blob/main/CONTRIBUTING.md) guide for more details on getting started with contributing to this project.

# License

Openraft is licensed under the terms of the [MIT License](https://en.wikipedia.org/wiki/MIT_License#License_terms) or the [Apache License 2.0](http://www.apache.org/licenses/LICENSE-2.0), at your choosing.

[`change_membership()`]: https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html#method.change_membership
[`add_learner()`]: https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html#method.add_learner
[`purge_log()`]: https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html#method.purge_log

[`enable_heartbeat`]: https://docs.rs/openraft/latest/openraft/struct.Config.html#structfield.enable_heartbeat
[`enable_elect`]: https://docs.rs/openraft/latest/openraft/struct.Config.html#structfield.enable_elect


<!-- TODO: update these API when 0.9 is published: -->

[`trigger_elect()`]: https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html#method.trigger_elect
[`trigger_snapshot()`]: https://docs.rs/openraft/latest/openraft/raft/struct.Raft.html#method.trigger_snapshot

<!--
TODO: enable when 0.9 is published:

[`ensure_linearizable()`]: xx
-->
