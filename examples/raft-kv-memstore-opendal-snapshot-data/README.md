# Remote Snapshot Storage with OpenDAL

Demonstrates storing and retrieving Raft snapshots in remote storage using [OpenDAL](https://docs.rs/opendal), enabling snapshot backup and disaster recovery patterns.

## Key Features Demonstrated

- **Remote snapshot storage**: Store snapshots in S3, OSS, or other remote backends via OpenDAL
- **`RaftNetworkV2` usage**: Full snapshot transmission without chunking
- **Snapshot backup patterns**: Implement snapshot archival and restoration
- **Minimal example**: Focuses on snapshot handling, other aspects simplified

## Overview

This example extends `raft-kv-memstore` to show how to:
- Store snapshot data in remote storage (e.g., S3, OSS)
- Transmit full snapshots between nodes
- Retrieve snapshots from remote storage

Useful for:
- Snapshot backups to cloud storage
- Disaster recovery scenarios
- Reducing local storage requirements

## Key Implementation Points

**Sending snapshots**: See `RaftNetworkV2::full_snapshot()` implementation
- Sends complete snapshot in single RPC
- Fetches from remote storage before transmission

**Receiving snapshots**: See `api::snapshot()` implementation
- Stores received snapshot to remote storage
- Installs snapshot in state machine

## Running

```bash
cargo test -- --nocapture
```

## Architecture

**Storage flow**:
1. State machine creates snapshot
2. Snapshot uploaded to remote storage (OpenDAL)
3. Leader fetches from remote storage when needed
4. Full snapshot sent to follower
5. Follower stores to its remote storage

## Comparison

| Feature | opendal-snapshot | standard |
|---------|------------------|----------|
| Snapshot location | Remote (S3/OSS) | Local disk |
| Backup capability | Built-in | Manual |
| Network transmission | Full snapshot | Chunked |