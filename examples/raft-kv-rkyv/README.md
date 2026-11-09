# Archive-backed Raft entries with rkyv

This runnable, three-node KV example uses **application-owned rkyv archives**
through OpenRaft's existing `Entry`, `Payload`, storage, and network abstractions.
It requires no rkyv dependency, derives, or new serialized-entry API in OpenRaft.
It illustrates the approach discussed in [#880] and [#479], and adds the persistent
log-store and restart/recovery demonstration requested in [#2290].

```bash
cargo run --manifest-path examples/raft-kv-rkyv/Cargo.toml
make test-example-raft-kv-rkyv

# Persistent mode: use an existing directory on a local Unix filesystem.
mkdir -p /tmp/raft-rkyv
cargo run --manifest-path examples/raft-kv-rkyv/Cargo.toml -- --data-dir /tmp/raft-rkyv
```

Persistent mode additionally stops node 3, drops its log store and state machine,
reopens its disk image, and prints `restarted node 3: hello = archive-backed world`.
Running it again against the same directory recovers all three nodes, elects node
1 explicitly, and writes another value. Ticking, automatic elections, heartbeats,
automatic leader restoration, and automatic snapshots are disabled to make the
example deterministic.

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
The application-owned [`LogStore`](src/log_store.rs) retains these entries and
returns clones of archive handles to replication and apply. Without `--data-dir`
it is memory-only; with that flag it durably publishes a complete disk image on
each mutation.

## Storage → network → follower storage → apply

```text
Command --rkyv::to_bytes--> ArchiveBuffer
    → custom Payload and Entry
    → leader's archive-backed log store
    → Vec<C::Entry> in AppendEntriesRequest
    → versioned bincode frame containing the existing archive bytes
    → fresh aligned buffer, archive validation
    → follower's archive-backed log store
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

## Persistent storage and recovery

[`DiskState`](src/disk_state.rs) frames native log IDs, votes, committed progress,
purged-log state, and archived entry bytes in a versioned bincode image. Reopening
constructs native control metadata and copies payload bytes into validated aligned
buffers; it never reconstructs `Command`. Subsequent log reads share those buffers.
An entry handle remains valid after purge, truncation, file replacement, or closing
the store because it owns its archive allocation. This is **not memory mapping**.

[`AtomicFile`](src/atomic_file.rs) implements a deliberately simple publication
protocol under an exclusive advisory lock on the node's `LOCK` file:

1. Fence the store, apply the mutation to the in-memory image, and encode it.
2. Write the next complete image to `log-state.next`.
3. `sync_all` the temporary file.
4. Rename it over `log-state` in the same directory.
5. `sync_all` the directory, then lift the fence and acknowledge.

Mutation and publication run in one blocking job, so cancellation of the async
caller cannot separate the in-memory image from the published one. The job changes
the in-memory image in place instead of copying it. Readers wait on the store lock
while the job runs, and a failed job leaves the store fenced, so no reader sees a
change that is not durable. Memory-only mode mutates in place without a blocking
job. Cloned store handles share the lock and serialized publication. Open a store
once per node directory; a second independent opener is rejected. Advisory locking
assumes cooperative writers; do not remove `LOCK` or move a node directory while a
store is open. The caller must provision an existing, durably created parent
directory; the cluster helper creates and syncs the per-node directories before
starting Raft.

On reopen, the store first syncs the locked directory to stabilize any rename left
by a previous process. Only `log-state` is authoritative. An unpublished `.next`
file is ignored and overwritten by the next publication, including after an interrupted first
write. A published image with an unknown format, invalid checksum, truncated frame,
invalid archive, or trailing bytes is rejected, not silently replaced with an empty
store. The image header `ORKYV001` fixes the application schema, rkyv settings, and
native metadata schema. Changing any of them needs a new version and explicit
migration; no migration is implemented. CRC32 detects accidental corruption, not
malicious tampering. Images are capped at 64 MiB, including the header.

Successful append callbacks, vote saves, committed-progress saves, truncation, and
purge wait for both file and directory sync. An encoding or publication error leaves
the live store fenced: drop **all** handles and reopen it. Before rename, recovery sees the previous
image; after rename but before successful directory sync, a real crash may recover
either complete generation. The operation has not been acknowledged, so either is
permitted, but recovery must never lose previously acknowledged durable data.
Filesystem/hardware support for atomic same-directory rename and reliable sync is
required. This reference example targets local Unix filesystems, not network shares.

### State-machine and snapshot boundary

Only the **log store** is persistent. Applied state and snapshots remain volatile.
The cluster disables automatic snapshots/purging and retains the entire log, saves
committed progress, and replays the committed prefix into a fresh state machine at
restart. A restarted follower then obtains later entries through normal replication.
It is safe to discard volatile application state only because the complete committed
history is retained.

Do not manually purge the cluster's logs or rely on an installed volatile snapshot
for recovery. Once a prefix is purged, a durable snapshot or equivalent durable
state machine must be published before those entries are removed. The low-level
store implements purge for the normal storage contract and its tests, but this
example does not provide a persistent snapshot/state-machine implementation.

The full-image strategy rewrites every retained archive for every mutation, including
metadata updates. It is bounded, easy to audit, and intentionally unsuitable for
high-throughput production use. A WAL or segmented/mapped store can improve write
amplification while preserving the same entry/payload and recovery contracts.

## What the tests establish

- A three-voter cluster replicates a large archived value through the byte codec.
- Reading a local log and applying it share the same archive allocation; followers
  have independent allocations with byte-identical archives.
- Cloning an archive and changing an entry's log ID do not copy its payload.
- Snapshot round trips preserve archived values and membership/applied metadata.
- Malformed/truncated archives, unknown wire versions, and trailing bytes are
  rejected rather than exposed as archived references.
- The persistent store passes OpenRaft's storage test suite.
- Reopen preserves votes, committed progress, log IDs, suffix replacement, and
  purge/truncation metadata; reopened payloads are aligned and share handles locally.
- Failure injection interrupts partial writes, file-sync boundaries, rename, and
  directory-sync completion. Failed appends are not acknowledged, live stores are
  fenced, and recovery retains acknowledged data while ignoring unpublished images.
  These deterministic injected failures model publication boundaries, **not actual
  power loss or hardware/filesystem reordering**.
- Corrupt published frames, unsupported disk versions, and malformed archive bytes
  with a recomputed frame checksum are rejected on reopen.
- A voter restarts with a fresh state machine, replays its persisted committed prefix,
  and catches up after missing a write. A full cluster restart also recovers values.
- Retained archive handles survive log removal, file replacement, and store shutdown.
- Cancelling an async mutation does not split the in-memory image from the published
  one, and a mutation that stops before publication leaves the store fenced.

## Limits and compatibility

This demonstrates **avoiding reconstruction of application objects**, not
zero-copy kernel I/O or zero allocations. Encoding copies archives into a frame;
decoding uses a temporary byte vector and copies into aligned storage. A real
transport can optimize these copies independently. Raft still constructs entry
vectors and native metadata, and safe archive access has validation cost.

Without `--data-dir`, logs, state, and snapshots are **memory-only**. Persistent mode
adds the log-store durability described above, but not disk-backed archive mapping,
persistent applied state/snapshots, or a production storage implementation.

The example pins rkyv 0.8.18 with little-endian, 32-bit relative pointers. The
application-owned envelope has version 1 and an 8 MiB body limit. This is a
single-schema demonstration: reject incompatible versions, and design explicit
schema migration/version negotiation before supporting rolling upgrades or
reopening archives after schema changes. Bincode metadata also needs a versioned
schema; byte validation is not schema evolution. Large snapshots need chunking
rather than this whole-frame demo transport. The default sequential
`stream_append` implementation preserves response order.

[#880]: https://github.com/databendlabs/openraft/issues/880
[#479]: https://github.com/databendlabs/openraft/issues/479
[#2290]: https://github.com/databendlabs/openraft/issues/2290
