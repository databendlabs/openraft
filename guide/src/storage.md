Storage
=======
The way that data is stored and represented is an integral part of every data storage system. Whether it is a SQL or NoSQL database, a KV store, an AMQP / Streaming / Eventing system, a Graph database, or anything which stores data — control over the storage technology and technique is critical. This implementation of Raft uses the `RaftStorage` trait to define the behavior needed of an application's storage layer to work with Raft.

### implementation
There are a few important decisions which need to be made in order to implement the `RaftStorage` trait.

1. **How do you plan on storing your snapshots?** The `RaftStorage::Snapshot` associated type must declare the type your application uses for dealing with the raw bytes of a snapshot. For most applications, it stands to reason that a simple on-disk file is what will be used. As such, take a look at [Tokio's fs::File](https://docs.rs/tokio/latest/tokio/fs/struct.File.html). It satisfies all of the trait bounds for the `Snapshot` associated type.
2. **How do you plan on storing your data?** A majority of the methods of your `RaftStorage` impl will involve reading and writing data. Rust has a few data storage crates available to choose from which will satisfy these requirements. Have a look at [Sled](https://docs.rs/sled/latest/sled/), or [RocksDB](https://docs.rs/rocksdb/latest/rocksdb/). There are others to choose from, but these may be a solid starting point. Or you could always roll your own.

Once you're ready to begin with your implementation, be sure to adhere to the documentation of the `RaftStorage` methods themselves. There are plenty of data safety requirements to uphold in order for your application to work properly overall, and to work properly with Raft.

For inspiration, have a look at this [repo's `memstore` project](https://github.com/async-raft/async-raft/tree/master/memstore). It is an in-memory implementation of the `RaftStorage` trait, intended for demo and testing purposes.

### compaction / snapshots
This implementation of Raft automatically triggers log compaction based on runtime configuration, using the [`RaftStorage::do_log_compaction`](https://docs.rs/async-raft/latest/async_raft/storage/trait.RaftStorage.html#tymethod.do_log_compaction) method. Everything related to compaction / snapshots starts with this method. Though snapshots are originally created in the [`RaftStorage::do_log_compaction`](https://docs.rs/async-raft/latest/async_raft/storage/trait.RaftStorage.html#tymethod.do_log_compaction) method, the Raft cluster leader may stream a snapshot over to other nodes if the node is new and needs to be brought up-to-speed, or if a node is lagging behind. Internally, Raft uses the `RaftStorage::Snapshot` associated type to work with the snapshot locally and for streaming to follower nodes.

Compaction / snapshotting are not optional in this system. It is an integral component of the Raft spec, and `RaftStorage` implementations should be careful to implement the compaction / snapshotting related methods carefully according to the trait's documentation.

When performing log compaction, the compaction can only cover the breadth of the log up to the last applied log and under write load this value may change quickly. As such, the storage implementation should export/checkpoint/snapshot its state machine, and then use the value of that export's last applied log as the metadata indicating the breadth of the log covered by the snapshot.

----

There is more to learn, so let's keep going. Time to learn about the most central API of this project.
