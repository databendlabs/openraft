<h1 align="center">async raft</h1>
<div align="center">
    <strong>
        An implementation of the <a href="https://raft.github.io/">Raft distributed consensus protocol</a> using <a href="https://tokio.rs/">the Tokio framework</a>. Please ⭐ on <a href="https://github.com/async-raft/async-raft">github</a>!
    </strong>
</div>
<br/>
<div align="center">

[![Build Status](https://github.com/async-raft/async-raft/workflows/ci/badge.svg?branch=master)](https://github.com/async-raft/async-raft/actions)
[![Discord Chat](https://img.shields.io/discord/845414467234693170?logo=discord&style=flat-square)](https://discord.gg/DYSDaBjwaA)
[![Crates.io](https://img.shields.io/crates/v/async-raft.svg)](https://crates.io/crates/async-raft)
[![docs.rs](https://docs.rs/async-raft/badge.svg)](https://docs.rs/async-raft)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE)
![Crates.io](https://img.shields.io/crates/d/async-raft.svg)
![Crates.io](https://img.shields.io/crates/dv/async-raft.svg)

</div>
<br/>

---


## From datafuse developer:

**This fork fixed several bugs in the original async-raft, introduced new features
and is matinained by datafuse team. The API and internal data structure has changed
to fit the latest [datafuse](https://github.com/datafuselabs/datafuse) need.**

The tags in form of `v0.6.2-alpha.*` are datafuse maintained versions.
They are well tested but it should still be considered as a beta.

Bug fixes are:

- fix: handle-vote should compare last_log_id in dictionary order, not in vector order
- fix: race condition of concurrent snapshot-install and apply.
- fix: a non-voter not in joint config should not block replication
- fix: when append-entries, deleting entries after prev-log-id causes committed entry to be lost
- fix: too many(50) inconsistent log should not live lock append-entries
- fix: RaftCore.entries_cache is inconsistent with storage. removed it.
- fix: snapshot replication does not need to send a last 0 size chunk
- fix: install snapshot req with offset GE 0 should not start a new session.
- fix: leader should not commit when there is no replication to voters.
- fix: after 2 log compaction, membership should be able to be extract from prev compaction log
- fix: when finalize_snapshot_installation, memstore should not load membership from its old log that are going to be overridden by snapshot.
- fix: leader should re-create and send snapshot when `threshold/2 < last_log_index - snapshot < threshold`
- fix: client_read has using wrong quorum=majority-1
- fix: doc-include can only be used in nightly build
- fix: when handle_update_match_index(), non-voter should also be considered, because when member change a non-voter is also count as a quorum member
- fix: when calc quorum, the non-voter should be count
- fix: a NonVoter should stay as NonVoter instead of Follower after restart
- fix: discarded log in replication_buffer should be finally sent.
- fix: a conflict is expected even when appending empty enties
- fix: last_applied should be updated only when logs actually applied.

If you'd like to use this repo, notice the changes to the APIs:
It is not quite difficult to follow up the changes.
Most changes to the APIs are obvious and are some generalized form of the original ones.
See the commits starts with `change:`

- change: MembershipConfig.member type is changed form HashSet BTreeSet
- change: pass all logs to apply_entry_to_state_machine(), not just Normal logs.
- change: use LogId to track last applied instead of using just an index.
- change: reduce one unnecessary snapshot serialization
- change: add CurrentSnapshotData.meta: SnapshotMeta, which is a container of all meta data of a snapshot: last log id included, membership etc.
- change: Entry: merge term and index to log_id: LogId
- change: InitialState: last_log_{term,index} into last_log: LogId
- change: AppendEntriesRequest: merge prev_log_{term,index} into prev_log: LogId
- change: RaftCore: merge last_log_{term,index} into last_log: LogId
- change: RaftCore: replace `snapshot_index` with `snapshot_last_included: LogId`. Keep tracks of both snapshot last log term and index.
- change: CurrentSnapshotData: merge `term` and `index` into `included`.
- change: use snapshot-id to identify a snapshot stream
- change: InstallSnapshotRequest: merge last_included{term,index} into last_included



---

Blazing fast Rust, a modern consensus protocol, and a reliable async runtime — this project intends to provide a consensus backbone for the next generation of distributed data storage systems (SQL, NoSQL, KV, Streaming, Graph ... or maybe something more exotic).

[The guide](https://async-raft.github.io/async-raft) is the best place to get started, followed by [the docs](https://docs.rs/async-raft/latest/async_raft/) for more in-depth details.

This crate differs from other Raft implementations in that:
- It is fully reactive and embraces the async ecosystem. It is driven by actual Raft events taking place in the system as opposed to being driven by a `tick` operation. Batching of messages during replication is still used whenever possible for maximum throughput.
- Storage and network integration is well defined via two traits `RaftStorage` & `RaftNetwork`. This provides applications maximum flexibility in being able to choose their storage and networking mediums. See the [storage](https://async-raft.github.io/async-raft/storage.html) & [network](https://async-raft.github.io/async-raft/network.html) chapters of the guide for more details.
- All interaction with the Raft node is well defined via a single public `Raft` type, which is used to spawn the Raft async task, and to interact with that task. The API for this system is clear and concise. See the [raft](https://async-raft.github.io/async-raft/raft.html) chapter in the guide.
- Log replication is fully pipelined and batched for optimal performance. Log replication also uses a congestion control mechanism to help keep nodes up-to-date as efficiently as possible.
- It fully supports dynamic cluster membership changes according to the Raft spec. See the [`dynamic membership`](https://async-raft.github.io/async-raft/dynamic-membership.html) chapter in the guide. With full support for leader stepdown, and non-voter syncing.
- Details on initial cluster formation, and how to effectively do so from an application's perspective, are discussed in the [cluster formation](https://async-raft.github.io/async-raft/cluster-formation.html) chapter in the guide.
- Automatic log compaction with snapshots, as well as snapshot streaming from the leader node to follower nodes is fully supported and configurable.
- The entire code base is [instrumented with tracing](https://docs.rs/tracing/). This can be used for [standard logging](https://docs.rs/tracing/latest/tracing/index.html#log-compatibility), or for [distributed tracing](https://docs.rs/tracing/latest/tracing/index.html#related-crates), and the verbosity can be [statically configured at compile time](https://docs.rs/tracing/latest/tracing/level_filters/index.html) to completely remove all instrumentation below the configured level.

This implementation strictly adheres to the [Raft spec](https://raft.github.io/raft.pdf) (*pdf warning*), and all data models use the same nomenclature found in the spec for better understandability. This implementation of Raft has integration tests covering all aspects of a Raft cluster's lifecycle including: cluster formation, dynamic membership changes, snapshotting, writing data to a live cluster and more.

If you are building an application using this Raft implementation, open an issue and let me know! I would love to add your project's name & logo to a users list in this project.

### contributing
Check out the [CONTRIBUTING.md](https://github.com/async-raft/async-raft/blob/master/CONTRIBUTING.md) guide for more details on getting started with contributing to this project.

### license
async-raft is licensed under the terms of the MIT License or the Apache License 2.0, at your choosing.

----

**NOTE:** the appearance of the "section" symbols `§` throughout this project are references to specific sections of the Raft spec.
