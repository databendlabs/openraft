# types-kv

Shared KV request/response types for openraft example crates.

## Types

- `Request` - KV store write operations (`Set` and version-based `CompareAndSet`)
- `Response` - KV store operation results with an optional versioned value
- `VersionedValue` - a value and its version, the log index of the write that stored it
