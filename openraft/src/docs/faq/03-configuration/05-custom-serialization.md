### Can I use a custom serialization format like bitcode instead of serde?

Yes. OpenRaft uses abstract types via [`RaftTypeConfig`][], so you can use any
serialization format — bitcode, bincode, flatbuffers, or anything else — without
changes to OpenRaft itself.

**How it works**:

OpenRaft does not dictate how data is serialized on the wire or in storage. The
associated types on [`RaftTypeConfig`][] — [`D`][`RaftTypeConfig::D`],
[`R`][`RaftTypeConfig::R`], [`NodeId`][`RaftTypeConfig::NodeId`],
[`Node`][`RaftTypeConfig::Node`], [`LeaderId`][`RaftTypeConfig::LeaderId`],
[`Vote`][`RaftTypeConfig::Vote`], and [`Entry`][`RaftTypeConfig::Entry`] — only
require trait bounds like `Debug`, `Clone`, and `Send`. Serialization is entirely
your responsibility.

**Steps**:

1. Define your own types that derive your serialization framework's traits
   (e.g., bitcode's `Encode`/`Decode`).

2. Assign them to the [`RaftTypeConfig`][] associated types.

3. In your [`RaftNetworkV2`][] implementation, serialize and deserialize
   request/response types using your chosen format. The network layer is fully
   under your control.

The `serde` feature flag in OpenRaft only adds `serde::Serialize`/`serde::Deserialize`
derives to the built-in types. If you provide your own types, you don't need to
enable the `serde` feature at all.

[`RaftTypeConfig`]: `crate::RaftTypeConfig`
[`RaftTypeConfig::D`]: `crate::RaftTypeConfig::D`
[`RaftTypeConfig::R`]: `crate::RaftTypeConfig::R`
[`RaftTypeConfig::NodeId`]: `crate::RaftTypeConfig::NodeId`
[`RaftTypeConfig::Node`]: `crate::RaftTypeConfig::Node`
[`RaftTypeConfig::LeaderId`]: `crate::RaftTypeConfig::LeaderId`
[`RaftTypeConfig::Vote`]: `crate::RaftTypeConfig::Vote`
[`RaftTypeConfig::Entry`]: `crate::RaftTypeConfig::Entry`
[`RaftNetworkV2`]: `crate::network::RaftNetworkV2`
