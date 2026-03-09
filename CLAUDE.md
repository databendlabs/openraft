# Openraft Development Guidelines

## Code Quality Checks

Always use `make` targets for checking and linting:

```bash
# Format and lint
make lint

# Full check: format, lint, tests, doc
make basic_check

# Run all tests
make test
```

**Never run `cargo clippy` or `cargo fmt` directly** - use the Makefile targets which ensure consistent settings across all workspace crates.

## Key Makefile Targets

- `make lint` - Format all crates and run clippy
- `make basic_check` - Full validation (lint + tests + doc)
- `make test` - Run all tests
- `make doc` - Build documentation

## Rust Style

- Always use `where` clauses for trait bounds instead of inline bounds.
  - Correct: `fn foo<T>(x: T) where T: RaftLeaderId`
  - Wrong: `fn foo<T: RaftLeaderId>(x: T)`
  - Applies to structs, impls, and trait definitions as well.

## Code Organization Convention

Each file should contain just one main trait or type, along with its associated implementations. This rule applies automatically when adding **new** types or traits — do not reorganize existing files unless explicitly asked.

## Project Structure

- `openraft/` - Core Raft implementation
- `legacy/` - Backward compatibility for deprecated APIs (formerly `network-v1`)
- `rt/`, `rt-tokio/` - Async runtime abstractions
- `stores/memstore/` - In-memory storage implementation
- `tests/` - Integration tests
- `examples/` - Example applications
