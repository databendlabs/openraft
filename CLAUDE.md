# OpenRaft Development Guidelines

## Code Quality Checks

Always use `make` targets for checking and linting:

```bash
# Completion gate: check format, Clippy, tests, doctests, and doc.
# Never writes to the worktree.
make verify

# Rewrite the worktree: Clippy fixes, typo fixes, formatting
make fix

# Run all tests
make test
```

Finish a change with `make verify`. Run `make fix` only when the change is
yours to rewrite: it edits files in place, so it must never run on a diff
someone else has already reviewed.

Do not run `cargo clippy` or `cargo fmt` directly. The Makefile targets apply the same settings across every workspace crate.

## Key Makefile Targets

Read-only:

- `make verify`: format check, Clippy, tests, doctests, and documentation build
- `make test`: run all tests
- `make doc`: build documentation

Modifies files:

- `make fix`: apply Clippy fixes, typo fixes, and formatting
- `make fmt`: format all crates
- `make lint`: format all crates and run Clippy

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

## Test Placement

Keep unit tests inline as `#[cfg(test)] mod tests` at the end of the file. Move them to their own file once they outgrow the code under test, or exceed roughly 200 lines.

A moved test module stays a child of the module it tests, so it keeps access to private items:

- Directory module: declare `#[cfg(test)] mod <topic>_test;` in `mod.rs` and put the file next to it.
- File module `foo.rs`: declare `#[cfg(test)] mod foo_test;` inside `foo.rs` and put the file at `foo/foo_test.rs`.

Both layouts are in use, and the choice follows test size, so a directory containing a mix of inline and separate test modules is expected rather than something to normalize.

## Test Style

- Keep short setup and action sequences inline, even when they occur in one or two tests. Extract a helper only when substantial reuse outweighs the extra abstraction level.
- Write each test as one linear sequence of purpose-driven phases.
- Introduce an independent phase with `tracing::info!` that states why the following actions are performed and includes useful runtime context such as the log index, node, or term.
- Put the phase in a scoped `{ ... }` block after its log entry so temporary values stay local and the phase boundary is visible. Use a block expression when a phase must return a value to later phases; phase blocks may be nested.
- Prefer runtime logging over standalone comments for explaining test actions.

## Project Structure

- `openraft/` - Core Raft implementation
- `legacy/` - Backward compatibility for deprecated APIs (formerly `network-v1`)
- `rt/`, `rt-tokio/` - Async runtime abstractions
- `stores/memstore/` - In-memory storage implementation
- `tests/` - Integration tests
- `examples/` - Example applications
