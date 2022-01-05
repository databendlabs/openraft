# Openraft

<strong>
    Advanced <a href="https://raft.github.io/">Raft</a> using <a href="https://tokio.rs/">the Tokio framework</a>. Please ⭐ on <a href="https://github.com/datafuselabs/openraft">github</a>!
</strong>

---

[![CI](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml/badge.svg)](https://github.com/datafuselabs/openraft/actions/workflows/ci.yaml)
[![Crates.io](https://img.shields.io/crates/v/openraft.svg)](https://crates.io/crates/openraft)
[![docs.rs](https://docs.rs/openraft/badge.svg)](https://docs.rs/openraft)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)
![Crates.io](https://img.shields.io/crates/d/openraft.svg)
![Crates.io](https://img.shields.io/crates/dv/openraft.svg)

Raft is not yet good enough.
This project intends to improve raft as the next generation consensus protocol for distributed data storage systems (SQL, NoSQL, KV, Streaming, Graph ... or maybe something more exotic).

Currently openraft is the consensus engine of meta-service cluster in [databend](https://github.com/datafuselabs/databend).


- **Get started**: [The guide](https://datafuselabs.github.io/openraft) is the best place to get started,
  followed by [the docs](https://docs.rs/openraft/latest/) for more in-depth details.

- **Openraft API is not stable yet.** Before `1.0.0`, an upgrade may contain incompatible changes.
  Check our [change-log](https://github.com/datafuselabs/openraft/blob/main/change-log.md)

- Openraft is derived from [async-raft](https://docs.rs/crate/async-raft/latest) with several bugs fixed:

<details>
  <summary>List of fixed bugs:</summary>


-   Fixed: [6c0ccaf3](https://github.com/datafuselabs/openraft/commit/6c0ccaf3ab3f262437d8fc021d4b13437fa4c9ac) consider joint config when starting up and committing.; by drdr xp; 2021-12-24
-   Fixed: [228077a6](https://github.com/datafuselabs/openraft/commit/228077a66d5099fd404ae9c68f0977e5f978f102) a restarted follower should not wait too long to elect. Otherwise, the entire cluster hangs; by drdr xp; 2021-11-19
-   Fixed: [a48a3282](https://github.com/datafuselabs/openraft/commit/a48a3282c58bdcae3309545a06431aecf3e65db8) handle-vote should compare last_log_id in dictionary order, not in vector order; by drdr xp; 2021-09-09
-   Fixed: [eed681d5](https://github.com/datafuselabs/openraft/commit/eed681d57950fc58b6ca71a45814b8f6d2bb1223) race condition of concurrent snapshot-install and apply.; by drdr xp; 2021-09-01
-   Fixed: [9540c904](https://github.com/datafuselabs/openraft/commit/9540c904da4ae005baec01868e01016f3bc76810) when append-entries, deleting entries after prev-log-id causes committed entry to be lost; by drdr xp; 2021-08-31
-   Fixed: [6d53aa12](https://github.com/datafuselabs/openraft/commit/6d53aa12f66ecd08e81bcb055eb17387b835e2eb) too many(50) inconsistent log should not live lock append-entries; by drdr xp; 2021-08-31
-   Fixed: [4d58a51e](https://github.com/datafuselabs/openraft/commit/4d58a51e41189acba06c1d2a0e8466759d9eb785) a non-voter not in joint config should not block replication; by drdr xp; 2021-08-31
-   Fixed: [8cd24ba0](https://github.com/datafuselabs/openraft/commit/8cd24ba0f0212e94e61f21a7be0ce0806fcc66d5) RaftCore.entries_cache is inconsistent with storage. removed it.; by drdr xp; 2021-08-23
-   Fixed: [2eccb9e1](https://github.com/datafuselabs/openraft/commit/2eccb9e1f82f1bf71f6a2cf9ef6da7bf6232fa84) install snapshot req with offset GE 0 should not start a new session.; by drdr xp; 2021-08-22
-   Fixed: [eee8e534](https://github.com/datafuselabs/openraft/commit/eee8e534e0b0b9abdb37dd94aeb64dc1affd3ef7) snapshot replication does not need to send a last 0 size chunk; by drdr xp; 2021-08-22
-   Fixed: [beb0302b](https://github.com/datafuselabs/openraft/commit/beb0302b4fec0758062141e727bf1bbcfd4d4b98) leader should not commit when there is no replication to voters.; by drdr xp; 2021-08-18
-   Fixed: [dba24036](https://github.com/datafuselabs/openraft/commit/dba24036cda834e8c970d2561b1ff435afd93165) after 2 log compaction, membership should be able to be extract from prev compaction log; by drdr xp; 2021-07-14
-   Fixed: [447dc11c](https://github.com/datafuselabs/openraft/commit/447dc11cab51fb3b1925177d13e4dd89f998837b) when finalize_snapshot_installation, memstore should not load membership from its old log that are going to be overridden by snapshot.; by drdr xp; 2021-07-13
-   Fixed: [cf4badd0](https://github.com/datafuselabs/openraft/commit/cf4badd0d762757519e2db5ed2f2fc65c2f49d02) leader should re-create and send snapshot when `threshold/2 < last_log_index - snapshot < threshold`; by drdr xp; 2021-07-08
-   Fixed: [d60f1e85](https://github.com/datafuselabs/openraft/commit/d60f1e852d3e5b9455589593067599d261f695b2) client_read has using wrong quorum=majority-1; by drdr xp; 2021-07-02
-   Fixed: [11cb5453](https://github.com/datafuselabs/openraft/commit/11cb5453e2200eda06d26396620eefe66b169975) doc-include can only be used in nightly build; by drdr xp; 2021-06-16
-   Fixed: [a10d9906](https://github.com/datafuselabs/openraft/commit/a10d99066b8c447d7335c7f34e08bd78c4b49f61) when handle_update_match_index(), non-voter should also be considered, because when member change a non-voter is also count as a quorum member; by drdr xp; 2021-06-16
-   Fixed: [d882e743](https://github.com/datafuselabs/openraft/commit/d882e743db4734b2188b137ebf20c0443cf9fb49) when calc quorum, the non-voter should be count; by drdr xp; 2021-06-02
-   Fixed: [6202138f](https://github.com/datafuselabs/openraft/commit/6202138f0766dcfd07a1a825af165f132de6b920) a conflict is expected even when appending empty entries; by drdr xp; 2021-05-24
-   Fixed: [f449b64a](https://github.com/datafuselabs/openraft/commit/f449b64aa9254d9a18bc2abb5f602913af079ca9) discarded log in replication_buffer should be finally sent.; by drdr xp; 2021-05-22
-   Fixed: [6d680484](https://github.com/datafuselabs/openraft/commit/6d680484ee3e352d4caf37f5cd6f57630f46d9e2) #112 : when a follower is removed, leader should stop sending log to it.; by drdr xp; 2021-05-21
-   Fixed: [89bb48f8](https://github.com/datafuselabs/openraft/commit/89bb48f8702762d33b59a7c9b9710bde4a97478c) last_applied should be updated only when logs actually applied.; by drdr xp; 2021-05-20
-   Fixed: [39690593](https://github.com/datafuselabs/openraft/commit/39690593a07c6b9ded4b7b8f1aca3191fa7641e4) a NonVoter should stay as NonVoter instead of Follower after restart; by drdr xp; 2021-05-14


A full list of changes/fixes can be found in [change-log](https://github.com/datafuselabs/openraft/blob/main/change-log.md)

</details>



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
