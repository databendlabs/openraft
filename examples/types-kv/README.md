# types-kv

Shared KV request/response types for openraft example crates.

## Types

- `Request` - KV store write operations (`Set` and version-based `CompareAndSet`)
- `Response` - KV store operation results with an optional versioned value
- `VersionedValue` - a value and its version, the log index of the write that stored it
- `RequestId` - a client ID and a serial that identify one write of one client
- `DedupRequest` - a `Request` tagged with a `RequestId`, which a state machine
  applies at most once; a resent copy gets the response of the first
  application
- `KvCommand` - what a state machine reads from a log entry's command: the
  `Request`, and its `RequestId` if it has one
