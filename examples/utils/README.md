# Example Utilities

Shared utilities for Openraft examples, reducing boilerplate across example implementations.

## Contents

- **`declare_types.rs`** - Type declarations for [`RaftTypeConfig`] implementations
  - Provides type aliases for common Raft types (Node, Entry, Response, etc.)
  - Reduces repetitive type definitions across examples
  - Usage: Import and use the generated type aliases
- **`append_reply.rs`** - `AppendReply`, a serializable wire format for OpenRaft's `AppendResult`
  - Keeps the log id the follower accepted and its conflict hint
  - Usage: Include it with `#[path = "../../utils/append_reply.rs"]` and encode it with the example's codec

## Purpose

This crate centralizes common type declarations used by multiple examples, making example code:
- More concise and readable
- Easier to maintain
- Consistent across different examples

## Usage

```rust
use example_utils::declare_raft_types;

declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
);
```

Used by all Openraft examples for type configuration.