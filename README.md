<div align="center">
    <h1>Openraft</h1>
    <h4>
        Advanced <a href="https://raft.github.io/">Raft</a> in 🦀 Rust using <a href="https://tokio.rs/">Tokio</a>. Please ⭐ on <a href="https://github.com/datafuselabs/openraft">github</a>!
    </h4>


[![Crates.io](https://img.shields.io/crates/v/openraft.svg)](https://crates.io/crates/openraft)
[![docs.rs](https://docs.rs/openraft/badge.svg)](https://docs.rs/openraft)
[![guides](https://img.shields.io/badge/guide-%E2%86%97-brightgreen)](https://datafuselabs.github.io/openraft)
<br/>
[![CI](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml/badge.svg)](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml)
![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)
![Crates.io](https://img.shields.io/crates/d/openraft.svg)
![Crates.io](https://img.shields.io/crates/dv/openraft.svg)

</div>

🪵🪵🪵 Raft is not yet good enough.
This project intends to improve raft as the next generation consensus protocol for distributed data storage systems (SQL, NoSQL, KV, Streaming, Graph ... or maybe something more exotic).

Currently openraft is the consensus engine of meta-service cluster in [databend](https://github.com/datafuselabs/databend).


- **Get started**: [The guide](https://datafuselabs.github.io/openraft) is the best place to get started,
  followed by [the docs](https://docs.rs/openraft/latest/) for more in-depth details.
- Openraft is derived from [async-raft](https://docs.rs/crate/async-raft/latest) with several bugs fixed: [Fixed bugs](https://github.com/datafuselabs/openraft/blob/main/derived-from-async-raft.md).

# Versions

- **Openraft API is not stable yet**. Before `1.0.0`, an upgrade may contain incompatible changes.
  Check our [change-log](https://github.com/datafuselabs/openraft/blob/main/change-log.md). A commit message starts with a keyword to indicate the modification type of the commit:

  - `Change:` if it introduces incompatible changes.
  - `Feature:` if it introduces compatible non-breaking new features.
  - `Fix:` if it just fixes a bug.

- **Branch [release-0.6](https://github.com/datafuselabs/openraft/tree/release-0.6)**: In this release branch, [v0.6.5](https://github.com/datafuselabs/openraft/tree/v0.6.5) is the latest
    published version. `release-0.6` won't accept new features but only bug fixes.

- **Branch [release-0.7](https://github.com/datafuselabs/openraft/tree/release-0.7)**: In this release branch, [v0.7.0-alpha.1](https://github.com/datafuselabs/openraft/tree/v0.7.0-alpha.1) is the latest version. `release-0.7` won't be published until the backward compatibility with `release-0.6` is ready.

- **Branch main** has been under active development.

# Roadmap

- [x] [Extended joint membership](https://datafuselabs.github.io/openraft/dynamic-membership.html#the-safe-to-relation)

- [ ] Reduce the complexity of vote and pre-vote: [get rid of pre-vote RPC](https://github.com/datafuselabs/openraft/discussions/15);

- [ ] Reduce confliction rate when electing;
  Allow leadership to be taken in one term by a node with greater node-id.

- [ ] Support flexible quorum, e.g.:[Hierarchical Quorums](https://zookeeper.apache.org/doc/r3.5.9/zookeeperHierarchicalQuorums.html)

- [ ] Consider introducing read-quorum and write-quorum,
  improve efficiency with a cluster with an even number of nodes.

- [ ] Goal performance is 1,000,000 put/sec.

    Bench history:
    - 2022 Jul 01: 41,000 put/sec; 23,255 ns/op;
    - 2022 Jul 07: 43,000 put/sec; 23,218 ns/op; Use `Progress` to track replication.
    - 2022 Jul 09: 45,000 put/sec; 21,784 ns/op; Batch purge applied log

    Run the benchmark: `make bench_cluster_of_3`

    Benchmark setting:
    - No network.
    - In memory store.
    - A cluster of 3 nodes on one server.
    - Single client.

<!--
   - - [ ] Consider to separate log storage and log order storage.
   -   Leader only determines and replicates the index of log entries, not log
   -   payload.
      -->


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
  See the [`dynamic membership`](https://datafuselabs.github.io/openraft/dynamic-membership.html) chapter in the guide.

- Details on initial cluster formation, and how to effectively do so from an application's perspective,
  are discussed in the [cluster formation](https://datafuselabs.github.io/openraft/cluster-formation.html) chapter in the guide.

- Automatic log compaction with snapshots, as well as snapshot streaming from the leader node to follower nodes is fully supported and configurable.

- The entire code base is [instrumented with tracing](https://docs.rs/tracing/).
  This can be used for [standard logging](https://docs.rs/tracing/latest/tracing/index.html#log-compatibility), or for [distributed tracing](https://docs.rs/tracing/latest/tracing/index.html#related-crates), and the verbosity can be [statically configured at compile time](https://docs.rs/tracing/latest/tracing/level_filters/index.html) to completely remove all instrumentation below the configured level.


# Who use it

- [yuyang0/rrqlite](https://github.com/yuyang0/rrqlite)
- [raymondshe/matchengine-raft](https://github.com/raymondshe/matchengine-raft)

# Contributing

Check out the [CONTRIBUTING.md](https://github.com/datafuselabs/openraft/blob/master/CONTRIBUTING.md) guide for more details on getting started with contributing to this project.

# License

Openraft is licensed under the terms of the MIT License or the Apache License 2.0, at your choosing.
