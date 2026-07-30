# dir-transfer

Transfer a flat directory of immutable files to a remote peer as an ordered stream of frames,
without fixing a concrete transport.

This crate defines the file-level protocol: the frame types, a sender that turns a directory into
a frame stream, and a receiver that rebuilds and validates the directory. The carrier is anything
that moves opaque frames reliably and in order: TCP, gRPC streaming, an HTTP body, a message
queue.

Built for shipping RocksDB checkpoint directories as OpenRaft snapshots (see
`examples/sm-rocks`), but generic over any checkpoint-style directory of immutable files.

## Protocol

A transfer is one ordered frame stream:

```text
Manifest, { FileStart, Chunk*, FileEnd }, End
```

with one `FileStart`/`Chunk`/`FileEnd` group per manifest entry, in manifest order.

- `Manifest` carries the format version and every file's name and size.
- File names are flat: no path separators, no `.` or `..`.
- `FileEnd` carries a CRC-64/XZ checksum of the complete file contents.
- Every violation fails with `io::ErrorKind::InvalidData`; a failed session is discarded and
  restarted from scratch.

## Transport contract

A transport implements `FrameSink` on the sending node and `FrameSource` on the receiving node,
choosing its own frame encoding; `send_dir()` and `recv_dir()` drive one complete session over
them. A conforming transport must:

1. Deliver the frames of one session to exactly one receiver, in order, without loss or
   duplication.
2. Propagate receiver errors back to the sender; on any error both sides drop the session.
3. Report success only after the receiver's `finish()` succeeds.

Retry, authentication, compression, and rate limiting are transport concerns, outside this
protocol.
