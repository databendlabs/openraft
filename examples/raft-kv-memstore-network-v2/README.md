# RaftNetworkV2 Example

Demonstrates using `RaftNetworkV2` for flexible snapshot data handling without file-like constraints.

## Key Features Demonstrated

- **`RaftNetworkV2` API**: Full snapshot transmission in single RPC
- **Flexible snapshot types**: Use any data type, not limited to `AsyncSeek + AsyncRead + AsyncWrite`
- **Custom snapshot data**: Send structured data directly without file format conversion
- **Minimal example**: Focuses on V2 network interface, other aspects simplified

## Overview

`RaftNetworkV2` provides an alternative to the chunked snapshot transmission in `RaftNetwork`:

**RaftNetwork (V1)**:
- Sends snapshots in chunks
- Requires file-like types (`AsyncSeek + AsyncRead + AsyncWrite + Unpin`)
- More complex implementation

**RaftNetworkV2**:
- Sends full snapshot in one RPC call
- Accepts any `SnapshotData` type
- Simpler implementation for in-memory snapshots

## Key Implementation Points

**Sending snapshots**: See `RaftNetworkV2::full_snapshot()` implementation
- Transmits complete snapshot data in single call
- No chunking or streaming required

**Receiving snapshots**: See `api::snapshot()` implementation
- Receives and installs full snapshot
- Handles snapshot data as single unit

## Running

```bash
cargo test -- --nocapture
```

## Comparison

| Feature | RaftNetworkV2 | RaftNetwork |
|---------|---------------|-------------|
| Snapshot transmission | Full (1 RPC) | Chunked (N RPCs) |
| Data type constraints | None | File-like traits |
| Implementation complexity | Simpler | More complex |
| Best for | In-memory snapshots | Large/streaming snapshots |