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


- **Get started**: [The guide](https://docs.rs/openraft/latest/openraft/docs/getting_started/index.html) is the best place to get started,
  followed by [the docs](https://docs.rs/openraft/latest/openraft/docs/index.html) for more in-depth details.

- [Openraft FAQ](https://docs.rs/openraft/latest/openraft/docs/faq/index.html) explains some common questions.

- 🙌 Questions? Join the [Discord channel](https://discord.gg/ZKw3WG7FQ9) or start a [discussion](https://github.com/datafuselabs/openraft/discussions/new).

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
  Latest published: [v0.8.7](https://github.com/datafuselabs/openraft/tree/v0.8.7) | [Change log v0.8.7](https://github.com/datafuselabs/openraft/blob/release-0.8/change-log.md#v087) |
  ⬆️  [0.7 to 0.8 upgrade guide](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_07_08/index.html) | ⬆️  [0.8.3 to 0.8.4 upgrade guide](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_083_084/index.html) |
  `release-0.8` **Won't** accept new features but only bug fixes.

- **Branch [release-0.7](https://github.com/datafuselabs/openraft/tree/release-0.7)**:
  Latest published: [v0.7.6](https://github.com/datafuselabs/openraft/tree/v0.7.6) | [Change log v0.7.6](https://github.com/datafuselabs/openraft/blob/release-0.7/change-log.md#v076) |
  ⬆️  [0.6 to 0.7 upgrade guide](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_06_07/index.html) |
  `release-0.7` **Won't** accept new features but only bug fixes.

- **Branch [release-0.6](https://github.com/datafuselabs/openraft/tree/release-0.6)**:
  Latest published: [v0.6.8](https://github.com/datafuselabs/openraft/tree/v0.6.8) | [Change log v0.6](https://github.com/datafuselabs/openraft/blob/release-0.6/change-log.md) |
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

Benchmark history:

|  Date      | clients | put/s         | ns/op      | Changes                              |
| :--        | --:     | --:           | --:        | :--                                  |
| 2023-04-26 | 256     | **1,014,000** |      985   |                                      |
| 2023-04-25 |  64     |   **730,000** |    1,369   | Split channels                       |
| 2023-04-24 |  64     |   **652,000** |    1,532   | Reduce metrics report rate           |
| 2023-04-23 |  64     |   **467,000** |    2,139   | State-machine moved to separate task |
|            |   1     |      70,000   | **14,273** |                                      |
| 2023-02-28 |   1     |      48,000   | **20,558** |                                      |
| 2022-07-09 |   1     |      45,000   | **21,784** | Batch purge applied log              |
| 2022-07-07 |   1     |      43,000   | **23,218** | Use `Progress` to track replication  |
| 2022-07-01 |   1     |      41,000   | **23,255** |                                      |


To access the benchmark, go to the `./cluster_benchmark` folder and run `make
bench_cluster_of_3`.

The benchmark is carried out with varying numbers of clients because:
- The `1 client` benchmark shows the average **latency** to commit each log.
- The `64 client` benchmark shows the maximum **throughput**.

The benchmark is conducted with the following settings:
- No network.
- In-memory store.
- A cluster of 3 nodes in a single process on a Mac M1-Max laptop.
- Request: empty
- Response: empty


# Features

- It is fully reactive and embraces the async ecosystem.
  It is driven by actual Raft events taking place in the system as opposed to being driven by a `tick` operation.
  Batching of messages during replication is still used whenever possible for maximum throughput.

- Storage and network integration is well defined via two traits `RaftStorage` & `RaftNetwork`.
  This provides applications maximum flexibility in being able to choose their storage and networking mediums.

- All interaction with the Raft node is well defined via a single public `Raft` type, which is used to spawn the Raft async task, and to interact with that task.
  The API for this system is clear and concise.

- Log replication is fully pipelined and batched for optimal performance.
  Log replication also uses a congestion control mechanism to help keep nodes up-to-date as efficiently as possible.

- It fully supports dynamic cluster membership changes with joint config.
  The buggy single-step membership change algo is not considered.
  See the [`dynamic membership`](https://docs.rs/openraft/latest/openraft/docs/cluster_control/dynamic_membership/index.html) chapter in the guide.

- Details on initial cluster formation, and how to effectively do so from an application's perspective,
  are discussed in the [cluster formation](https://docs.rs/openraft/latest/openraft/docs/cluster_control/cluster_formation/index.html) chapter in the guide.

- Automatic log compaction with snapshots, as well as snapshot streaming from the leader node to follower nodes is fully supported and configurable.

- The entire code base is [instrumented with tracing](https://docs.rs/tracing/).
  This can be used for [standard logging](https://docs.rs/tracing/latest/tracing/index.html#log-compatibility), or for [distributed tracing](https://docs.rs/tracing/latest/tracing/index.html#related-crates), and the verbosity can be [statically configured at compile time](https://docs.rs/tracing/latest/tracing/level_filters/index.html) to completely remove all instrumentation below the configured level.


# Who use it

- [yuyang0/rrqlite](https://github.com/yuyang0/rrqlite)
- [raymondshe/matchengine-raft](https://github.com/raymondshe/matchengine-raft)

# Contributing

Check out the [CONTRIBUTING.md](https://github.com/datafuselabs/openraft/blob/main/CONTRIBUTING.md) guide for more details on getting started with contributing to this project.

# License

Openraft is licensed under the terms of the [MIT License](https://en.wikipedia.org/wiki/MIT_License#License_terms) or the [Apache License 2.0](http://www.apache.org/licenses/LICENSE-2.0), at your choosing.
