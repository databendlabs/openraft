# Engine and Runtime Architecture

Openraft separates consensus logic (**Engine**) from I/O execution (**Runtime**).

## Architecture

The **Engine** (`engine/mod.rs`) contains the Raft algorithm as a pure state machine:
- Receives events (writes, votes, timeouts)
- Updates internal state
- Outputs `Command` instructions (`engine/command.rs`)

The **Runtime** (`runtime/mod.rs`) handles I/O:
- Executes commands via `RaftRuntime::run_command()`
- Interacts with storage and network
- Reports completion back to Engine as new events

This event-driven loop enables testing Engine logic without I/O dependencies.

## Write Operation Flow

This diagram shows how a client write request flows through the system:

```text
Client                    Engine                         Runtime      Storage      Network
  |                         |      write x=1                |            |            |
  |-------------------------------------------------------->|            |            |
  |        event:write      |                               |            |            |
  |        .------------------------------------------------|            |            |
  |        '--------------->|                               |            |            |
  |                         |      cmd:append-log-1         |            |            |
  |                         |--+--------------------------->|   append   |            |
  |                         |  |                            |----------->|            |
  |                         |  |                            |<-----------|            |
  |                         |  |   cmd:replicate-log-1      |   ok       |            |
  |                         |  '--------------------------->|            |            |
  |                         |                               |            |   send     |
  |                         |                               |------------------------>|
  |                         |                               |            |   send     |
  |                         |                               |------------------------>|
  |                         |                               |            |            |
  |                         |                               |<------------------------|
  |         event:ok        |                               |   ok       |            |
  |        .------------------------------------------------|            |            |
  |        '--------------->|                               |            |            |
  |                         |                               |<------------------------|
  |         event:ok        |                               |   ok       |            |
  |        .------------------------------------------------|            |            |
  |        '--------------->|                               |            |            |
  |                         |     cmd:commit-log-1          |            |            |
  |                         |------------------------------>|            |            |
  |                         |     cmd:apply-log-1           |            |            |
  |                         |------------------------------>|            |            |
  |                         |                               |   apply    |            |
  |                         |                               |----------->|            |
  |                         |                               |<-----------|            |
  |                         |                               |   ok       |            |
  |<--------------------------------------------------------|            |            |
  |        response         |                               |            |            |
```

## Flow Breakdown

1. **Write Request**: Client → Runtime → Engine receives write event

2. **Log Append**: Engine outputs `AppendInputEntries` command → Runtime persists to Storage

3. **Replication**: Engine outputs `Replicate` commands → Runtime sends to followers via Network

4. **Acknowledgment**: Followers respond → Runtime reports completion events to Engine

5. **Commit**: Engine determines quorum → Outputs `SaveCommitted` and `Apply` commands

6. **Apply**: Runtime executes state machine update → Responds to Client

## Benefits

**Testability**: Engine tests run without I/O - verify consensus logic with pure state transitions

**Clarity**: Raft algorithm isolated from storage/network implementation details

**Flexibility**: Swap Runtime implementations without touching consensus logic

## Command Types

Key command categories (see `Command` enum in `engine/command.rs`):

- **Storage**: `AppendInputEntries`, `TruncateLog`, `PurgeLog`, `SaveCommitted`
- **State Machine**: `Apply`, `StateMachine`
- **Network**: `Replicate`, `ReplicateCommitted`, `BroadcastHeartbeat`, `SendVote`
- **Membership**: `RebuildReplicationStreams`

Each command is executed by Runtime and may trigger new events back to Engine.
