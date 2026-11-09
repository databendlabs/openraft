# Archive-backed Raft entries with rkyv

This runnable, three-node KV example uses **application-owned rkyv archives**
through OpenRaft's existing `Entry`, `Payload`, storage, and network abstractions.
It requires no rkyv dependency, derives, or new serialized-entry API in OpenRaft.
It illustrates the approach discussed in [#880] and [#479].

```bash
cargo run --manifest-path examples/raft-kv-rkyv/Cargo.toml
make test-example-raft-kv-rkyv
```

The program initializes three voters, writes a value, waits for each node to
apply it, confirms a linearizable read, and prints:

```text
node 1: hello = archive-backed world
node 2: hello = archive-backed world
node 3: hello = archive-backed world
```

## Representation

- [`Command`](src/command.rs) is the application schema: a key and value. It
  implements rkyv `Archive` and `Serialize`, deliberately **not `Deserialize`**.
- [`ArchiveBuffer`](src/archive_buffer.rs) owns an immutable
  `Arc<rkyv::util::AlignedVec>`. Construction validates the archive at its final
  aligned address. Cloning a handle shares the allocation. Reading uses safe
  `rkyv::access` to obtain `&ArchivedCommand`, without reconstructing `Command`
  or its `String` fields; safe access revalidates the archive.
- [`Payload`](src/payload.rs) implements `RaftPayload`: normal data is an
  `ArchiveBuffer`, while blank entries and membership entries remain native.
- [`Entry`](src/entry.rs) implements `RaftEntry`: the log ID is ordinary native
  metadata, separate from the immutable application archive. Assigning a log ID
  does not rewrite the archive.

`RaftTypeConfig` selects these application types. `C::Entry` is an owning wrapper
around archived data, **not necessarily a fully deserialized application object**.
The [`log-mem`](../log-mem/) store retains these entries and returns clones of
archive handles to replication and apply.

## Storage → network → follower storage → apply

```text
Command --rkyv::to_bytes--> ArchiveBuffer
    → custom Payload and Entry
    → leader's log-mem store
    → Vec<C::Entry> in AppendEntriesRequest
    → versioned bincode frame containing the existing archive bytes
    → fresh aligned buffer, archive validation
    → follower's log-mem store
    → state machine reads ArchivedCommand in place
```

The transport is in-process to keep the example small, but it does **not** pass
requests directly between nodes or share archive buffers across nodes.
[`Connection`](src/connection.rs) serializes and decodes every request and reply.
`ArchiveBuffer` implements serde by forwarding its archived bytes; the outer
bincode envelope handles only Raft metadata, enum tags, and byte framing. Neither
networking nor follower apply deserializes `Command`.

Using serde for the envelope does not require using it for the application
schema. It satisfies OpenRaft's optional serde bounds while retaining the rkyv
representation of normal data. An application could instead define its own
metadata envelope with OpenRaft's serde feature disabled.

The state machine copies the key into its map but retains the archive handle as
the value. Snapshots frame the same archive bytes alongside native state metadata;
installation validates archives before replacing the state. No OpenRaft RPC,
error, membership, or log-ID type needs an `Archive` implementation.

## What the tests establish

- A three-voter cluster replicates a large archived value through the byte codec.
- Reading a local log and applying it share the same archive allocation; followers
  have independent allocations with byte-identical archives.
- Cloning an archive and changing an entry's log ID do not copy its payload.
- Snapshot round trips preserve archived values and membership/applied metadata.
- Malformed/truncated archives, unknown wire versions, and trailing bytes are
  rejected rather than exposed as archived references.

## Limits and compatibility

This demonstrates **avoiding reconstruction of application objects**, not
zero-copy kernel I/O or zero allocations. Encoding copies archives into a frame;
decoding uses a temporary byte vector and copies into aligned storage. A real
transport can optimize these copies independently. Raft still constructs entry
vectors and native metadata, and safe archive access has validation cost.

Logs, state, and snapshots are **memory-only**. This example provides no process
crash durability, disk-backed archive mapping, or persistent recovery claim.
For disk storage, retain backing allocations/mappings for the lifetime of every
entry and implement the normal durability and recovery contracts.

The example pins rkyv 0.8.18 with little-endian, 32-bit relative pointers. The
application-owned envelope has version 1 and an 8 MiB body limit. This is a
single-schema demonstration: reject incompatible versions, and design explicit
schema migration/version negotiation before supporting rolling upgrades or
persisting archives across schema changes. Bincode metadata also needs a versioned
schema; byte validation is not schema evolution. Large snapshots need chunking
rather than this whole-frame demo transport. The default sequential
`stream_append` implementation preserves response order.

[#880]: https://github.com/databendlabs/openraft/issues/880
[#479]: https://github.com/databendlabs/openraft/issues/479
