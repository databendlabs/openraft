# Single-Threaded Key-Value Store Example

Demonstrates using Openraft with non-`Send` types by enabling the `singlethreaded` feature flag.

## Key Features Demonstrated

- **Single-threaded operation**: No `Send` requirement for data types
- **Custom non-Send types**: `NodeId` and `Request` with `!Send` markers
- **Feature flag usage**: `singlethreaded` feature configuration

## Overview

This example shows how to build a Raft cluster when your application types cannot be sent across threads. Useful for:
- FFI integration with non-thread-safe libraries
- Performance optimization in single-threaded runtimes
- Types with thread-local state

## Implementation

Enable the feature in `Cargo.toml`:
```toml
openraft = { version = "0.10", features = ["singlethreaded"] }
```

Example non-`Send` types:
```rust
pub struct NodeId {
    pub id: u64,
    // Make it !Send
    _p: PhantomData<*const ()>,
}

pub enum Request {
    Set {
        key: String,
        value: String,
        // Make it !Send
        _p: PhantomData<*const ()>,
    }
}
```

## Running

```bash
cargo test -- --nocapture
```

## Comparison

| Feature | singlethreaded | standard |
|---------|---------------|----------|
| Send required | No | Yes |
| Multi-threaded | No | Yes |
| Use case | FFI, single-runtime | General purpose |