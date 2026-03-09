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

- Always use `where` clauses for trait bounds instead of inline bounds. This applies everywhere: functions, methods, structs, enums, impls, and trait definitions.
  - Correct: `fn foo<T>(x: T) where T: RaftLeaderId`
  - Wrong: `fn foo<T: RaftLeaderId>(x: T)`
  - Correct: `fn partial_cmp<V>(&self, other: &V) -> Option<Ordering> where V: RaftVote`
  - Wrong: `fn partial_cmp<V: RaftVote>(&self, other: &V) -> Option<Ordering>`

- Use proper trait bounds on structs, not expanded individual bounds. When a trait (e.g., `RaftLeaderId`) implies a set of bounds (`Debug + Display + Clone + ...`), use the trait name directly instead of listing the individual bounds.
  - Correct: `struct Foo<T> where T: RaftLeaderId`
  - Wrong: `struct Foo<T> where T: OptionalFeatures + PartialOrd + Eq + Clone + Debug + Display + 'static`

## Public API Change Annotations

Any change to a public type, trait, or associated type must have a `#[since]` attribute describing the change. Use `#[since(version = "0.10.0", change = "description")]` placed above any existing `#[since]` attribute.

- Add `#[since]` for: new public items, changed struct generic parameters, changed trait bounds on associated types, new associated types.
- Do NOT add `#[since]` on methods whose signatures only changed as a mechanical consequence of a parent type's generic parameter change (e.g., `Vote<LID>` methods that just replaced `C::Term` with `LID::Term`).
- `pub(crate)` items do not need `#[since]`.

## Code Organization Convention

Each file should contain just one main trait or type, along with its associated implementations. This rule applies automatically when adding **new** types or traits — do not reorganize existing files unless explicitly asked.

## Project Structure

- `openraft/` - Core Raft implementation
- `legacy/` - Backward compatibility for deprecated APIs (formerly `network-v1`)
- `rt/`, `rt-tokio/` - Async runtime abstractions
- `stores/memstore/` - In-memory storage implementation
- `tests/` - Integration tests
- `examples/` - Example applications
