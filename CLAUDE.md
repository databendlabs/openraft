# OpenRaft Development Guidelines

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

Do not run `cargo clippy` or `cargo fmt` directly. The Makefile targets apply the same settings across every workspace crate.

## Key Makefile Targets

- `make lint`: format all crates and run Clippy
- `make basic_check`: run linting, tests, and documentation checks
- `make test`: run all tests
- `make doc`: build documentation

## Rust Style

- Use `where` clauses for trait bounds instead of inline bounds. This applies to functions, methods, structs, enums, `impl` blocks, and trait definitions.
  - Correct: `fn foo<T>(x: T) where T: RaftLeaderId`
  - Wrong: `fn foo<T: RaftLeaderId>(x: T)`
  - Correct: `fn partial_cmp<V>(&self, other: &V) -> Option<Ordering> where V: RaftVote`
  - Wrong: `fn partial_cmp<V: RaftVote>(&self, other: &V) -> Option<Ordering>`

- Use the narrowest named trait bound available. When a trait such as `RaftLeaderId` implies `Debug + Display + Clone + ...`, use the trait instead of expanding its component bounds.
  - Correct: `struct Foo<T> where T: RaftLeaderId`
  - Wrong: `struct Foo<T> where T: OptionalFeatures + PartialOrd + Eq + Clone + Debug + Display + 'static`

## Public API Change Annotations

Any change to a public type, trait, or associated type needs a `#[since]` attribute. Add `#[since(version = "0.10.0", change = "description")]` above any existing `#[since]` attribute.

- Add `#[since]` for new public items, changed struct generic parameters, changed trait bounds on associated types, and new associated types.
- Do not add `#[since]` to methods whose signatures changed only as a mechanical consequence of a parent type's generic parameter change, such as `Vote<LID>` methods that replace `C::Term` with `LID::Term`.
- `pub(crate)` items do not need `#[since]`.

## Code Organization Convention

Each file should contain one main trait or type and its implementations. Apply this rule when adding new types or traits. Do not reorganize existing files unless explicitly asked.

## Project Structure

- `openraft/` - Core Raft implementation
- `legacy/` - Backward compatibility for deprecated APIs (formerly `network-v1`)
- `rt/`, `rt-tokio/` - Async runtime abstractions
- `stores/memstore/` - In-memory storage implementation
- `tests/` - Integration tests
- `examples/` - Example applications
