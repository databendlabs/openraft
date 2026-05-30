# openraft-fjall-kv-example

A fjall-backed persistent storage implementation for Openraft, demonstrating production-ready log storage and state machine patterns.

## Key Features Demonstrated

- **Persistent storage**: [`RaftLogStorage`] and [`RaftStateMachine`] with fjall
- **Column families**: Separate storage for logs, state machine, and metadata
- **Durability**: On-disk persistence for cluster recovery
- **Performance**: Efficient batch operations and compaction

## Overview

This example implements:
- **[`RaftLogStorage`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftLogStorage.html)** - Persistent Raft log storage
- **[`RaftStateMachine`](https://docs.rs/openraft/latest/openraft/storage/trait.RaftStateMachine.html)** - Persistent application state machine

Built with [fjall v3](https://fjall-rs.github.io/) for production-grade durability and performance. 

## Architecture

**Storage structure**:
- Logs stored in dedicated fjall key space
- State machine data in separate key space
- Vote and metadata persisted independently

**Asynchronous I/O operations**:
- WAL flush operations run in spawned tasks to avoid blocking the async runtime
- `save_vote()` and `append_to_log()` spawn async tasks for disk persistence
- Callbacks receive actual flush results for proper error propagation
- Log truncation (`purge()`) doesn't require immediate persistence

**Key Code Locations**:
- Storage implementation: `src/store.rs`
- Log storage with async WAL flush: `src/log_store.rs`
- Type definitions: See parent example for network and client implementations

## Comparison

| Feature | kv-fjall     | memstore |
|---------|--------------|----------|
| Storage | fjall (disk) | Memory |
| Persistence | Yes          | No |
| Recovery | Full         | None |
| Complexity | Higher       | Lower |

Built for testing and demonstration purposes.
