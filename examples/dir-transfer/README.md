# dir-transfer

Transfer a flat directory of immutable files to a remote peer as an ordered stream of frames,
without fixing a concrete transport.

This crate defines `DirFrame`, [`DirFrameProducer`](src/frame_producer.rs), and
[`DirWriter`](src/writer.rs). `DirFrameProducer::next_frame()` reads a directory as frames.
`DirWriter::write_frame()` validates and writes those frames into the target directory.
Call `DirWriter::finish()` after the `End` frame.

The carrier moves frames reliably and in order: TCP, gRPC streaming, an HTTP body, or a message
queue.

Built for shipping RocksDB checkpoint directories as OpenRaft snapshots (see
`examples/sm-rocks`), but generic over any checkpoint-style directory of immutable files.

## Protocol

A transfer is one ordered frame stream:

```text
Manifest, { FileStart, Chunk*, FileEnd }, End
```

with one `FileStart`/`Chunk`/`FileEnd` group per manifest entry, in manifest order.

- `Manifest` carries `PROTOCOL_VERSION` and every file's name and size as a `ManifestEntry`.
- File names are flat: at most `MAX_FILE_NAME_BYTES` bytes, no path separators, no `.` or `..`.
- `FileEnd` carries a CRC-64/XZ checksum of the complete file contents.
- Every violation fails with `io::ErrorKind::InvalidData`; a failed session is discarded and
  restarted from scratch.

## Transport contract

A transport implements `FrameSink` on the sending node and `FrameSource` on the receiving node,
choosing its own frame encoding; `send_dir()` and `recv_dir()` drive one complete session over
them. A conforming transport must:

1. Deliver the frames of one session to exactly one `DirWriter`, in order, without loss or
   duplication.
2. Propagate writer errors back to the sender; on any error both sides drop the session.
3. Report success only after `DirWriter::finish()` succeeds.

Retry, authentication, compression, and rate limiting are transport concerns, outside this
protocol.
