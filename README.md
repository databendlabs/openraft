actix raft
==========
[![Build Status](https://travis-ci.com/railgun-rs/actix-raft.svg?branch=master)](https://travis-ci.com/railgun-rs/actix-raft)

An implementation of the [Raft distributed consensus protocol](https://raft.github.io/) using the [Actix actor framework](https://github.com/actix/actix). Rust provides the best performance possible, Raft gives us distributed consensus, Actix gives us an outstanding actor model, this crate ties all of those elements together to help empower a wave of data storage systems which are distributed, memory safe, extremely fast, and memory efficient. Let's do this!

This implementation differs from other Raft implementations in that:
- It is fully reactive and embraces the async ecosystem. It is driven by actual Raft related events taking place in the system as opposed to being driven by a `tick` operation. Batching of messages during replication is still used whenever possible for maximum throughput.
- Storage and network integration is well defined via the two traits `RaftStorage` & `RaftNetwork`. This provides applications maximum flexibility in being able to choose their storage and networking mediums. This also allows for the storage interface to be synchronous or asynchronous based on the storage engine used, and allows for easy integration with the actix ecosystem's networking components for efficient async networking. See the [storage](https://railgun-rs.github.io/actix-raft/storage.html) & [network](https://railgun-rs.github.io/actix-raft/network.html) chapters of the guide.
- Submitting Raft RPCs & client requests to a running Raft node is also well defined via the Actix message types defined in the `messages` module in this crate. The API for this system is clear and concise. See the [raft](https://railgun-rs.github.io/actix-raft/raft.html) chapter in the guide.
- It fully supports dynamic cluster membership changes according to the Raft spec. See the [`dynamic membership`](https://railgun-rs.github.io/actix-raft/dynamix-membership.html) chapter in the guide.
- Details on initial cluster formation, and how to effectively do so from an application level perspective, are discussed in the [cluster formation](https://railgun-rs.github.io/actix-raft/cluster-formation.html) chapter in the guide.

This implementation strictly adheres to the [Raft spec](https://raft.github.io/raft.pdf) (*pdf warning*), and all data models use the same nomenclature found in the spec for better understandability. This implementation of Raft has integration tests covering all aspects of a Raft cluster's lifecycle including: cluster formation, dynamic membership changes, snapshotting, writing data to a live cluster and more.

### license
actix-raft is licensed under the terms of the MIT License or the Apache License 2.0, at your choosing.
