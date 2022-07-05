<h1 align="center">openraft</h1>
<div align="center">
    <strong>
        Advanced <a href="https://raft.github.io/">Raft</a> using <a href="https://tokio.rs/">the Tokio framework</a>. Please ⭐ on <a href="https://github.com/datafuselabs/openraft">github</a>!
    </strong>
</div>
<br/>
<div align="center">

[![CI](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml/badge.svg)](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml)
<!-- [![Crates.io](https://img.shields.io/crates/v/openraft.svg)](https://crates.io/crates/openraft) -->
<!-- [![docs.rs](https://docs.rs/openraft/badge.svg)](https://docs.rs/openraft) -->
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)
<!-- ![Crates.io](https://img.shields.io/crates/d/openraft.svg) -->
<!-- ![Crates.io](https://img.shields.io/crates/dv/openraft.svg) -->

</div>
<br/>

---

Raft is not yet good enough.
This project intends to improve raft as the next generation consensus protocol for distributed data storage systems (SQL, NoSQL, KV, Streaming, Graph ... or maybe something more exotic).

[The guide](https://datafuselabs.github.io/openraft) is the best place to get started.

Openraft is derived from [async-raft](https://docs.rs/crate/async-raft/latest) with several bugs fixed: [Fixed bugs](https://github.com/datafuselabs/openraft/blob/main/derived-from-async-raft.md).

A full list of changes/fixes can be found in [change-log](https://github.com/datafuselabs/openraft/blob/main/change-log.md)

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




# Contributing

Check out the [CONTRIBUTING.md](https://github.com/datafuselabs/openraft/blob/master/CONTRIBUTING.md) guide for more details on getting started with contributing to this project.

# License

Openraft is licensed under the terms of the MIT License or the Apache License 2.0, at your choosing.
