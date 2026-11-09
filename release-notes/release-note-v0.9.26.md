# Openraft v0.9.26

openraft now compiles for `wasm32-wasip1`. Native builds are unchanged: the API, the storage format and the tokio features openraft enables are the same as in 0.9.25, so upgrading from 0.9.25 only needs the version bump.

## Build for wasm32-wasip1 (#2269)

Until 0.9.25, openraft enabled tokio's `rt-multi-thread` feature on every target. tokio does not compile for wasm with that feature on, so a project that depended on openraft could not build for wasm. openraft runs on the caller's runtime; the only place it builds a multi-thread runtime is the storage test suite. 0.9.26 enables `rt-multi-thread` only when `target_family` is not `wasm`, the same condition tokio checks.

A wasm32-wasip1 build needs:

- **Cargo's feature resolver 2 or later.** It is the default for a package on edition 2021 or later, but a virtual workspace has to set `resolver = "2"` in its root `Cargo.toml`. Resolver 1 enables the features of target-specific dependencies on every target, which turns `rt-multi-thread` back on.
- **A current-thread tokio runtime**, such as `tokio::runtime::Builder::new_current_thread().enable_all().build()`. tokio has no multi-thread runtime on wasm.

If you run the storage test suite, `openraft::testing::Suite`, on wasm, each test case runs on a new current-thread runtime. The store under test must not call `tokio::task::block_in_place`, which panics on that runtime, and must not block the thread to wait for a task it spawned, because that task never runs.

0.10 has the equivalent change in #2272, released in v0.10.0-alpha.38.

Thanks to @wo1f for the fix.

## Crates

| Crate | Version | Changes since 0.9.25 |
|---|---|---|
| `openraft` | 0.9.26 | wasm32-wasip1 build |
| `openraft-macros` | 0.9.26 | none |
| `openraft-memstore` | 0.9.26 | none |

## For contributors

The `release-0.9` branch now pins `nightly-2026-11-08` in `rust-toolchain`, and every nightly CI job runs on it.

**Full change log**: [change-log/v0.9.26.md](https://github.com/databendlabs/openraft/blob/v0.9.26/change-log/v0.9.26.md), [v0.9.25...v0.9.26](https://github.com/databendlabs/openraft/compare/v0.9.25...v0.9.26)
