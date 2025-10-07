<!-- This page is rendered by docs.rs -->

<div align="center">
    <h1>Openraft</h1>
</div>

## API status

**Openraft API is currently unstable**.
Incompatibilities may arise in upgrades prior to `1.0.0`.
Refer to our [Change-Log](https://github.com/databendlabs/openraft/blob/main/change-log.md) for details.
[Upgrade Guides](crate::docs::upgrade_guide) explains how to upgrade.

Each commit message begins with a keyword indicating the type of change:

- `DataChange:` Changes to on-disk data types, possibly requiring manual upgrade.
- `Change:` Introduces incompatible API changes.
- `Feature:` Adds compatible, non-breaking new features.
- `Fix:` Addresses bug fixes.

## Feature flags

Openraft supports several feature flags to customize functionality:

**Default features**: `tokio-rt`, `adapt-network-v1`

**Runtime**:
- `tokio-rt` - Enable Tokio async runtime support (default)
- `singlethreaded` - Disallow sharing Raft instance across threads

**Serialization**:
- `serde` - Add `serde::Serialize` and `serde::Deserialize` bounds to data types

**Error handling**:
- `bt` - Enable error backtraces (requires nightly Rust)

**Networking**:
- `adapt-network-v1` - Automatically implement `RaftNetworkV2` for `RaftNetwork` implementations (default)

**Development**:
- `bench` - Enable benchmarks in unit tests (requires `RUSTC_BOOTSTRAP=1` with stable Rust)

**Compatibility**:
- `compat` - Provide basic compatible types for upgrades
- `type-alias` - Enable `openraft::alias::*` type shortcuts (unstable)

