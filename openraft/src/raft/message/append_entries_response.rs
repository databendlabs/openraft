use std::marker::PhantomData;

use openraft_macros::since;

use crate::RaftTypeConfig;

/// **REMOVED**: Use `openraft_legacy::network_v1::AppendEntriesResponse` instead.
///
/// The v1 `AppendEntriesResponse` has been moved to the `openraft-legacy` crate, next to the v1
/// `RaftNetwork` that returns it. This empty stub only names the new path in its deprecation
/// warning: it has no variants or conversions, so any other use fails to compile.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-legacy = "0.10.2"
/// ```
///
/// A transport that serializes the response with Serde enables the `serde` feature of
/// `openraft-legacy`, which also enables `openraft/serde`:
/// ```toml
/// [dependencies]
/// openraft-legacy = { version = "0.10.2", features = ["serde"] }
/// ```
///
/// The moved type derives `Serialize` and `Deserialize` only with this feature: `openraft/serde`
/// derived them before 0.10.2 and no longer does. A transport that encodes the response without
/// Serde does not need the feature.
///
/// Update imports:
/// ```ignore
/// use openraft_legacy::network_v1::AppendEntriesResponse;
/// ```
#[since(
    version = "0.10.2",
    change = "an empty deprecated stub: the type moved to `openraft_legacy::network_v1`"
)]
#[deprecated(
    since = "0.10.2",
    note = "AppendEntriesResponse has been moved to the `openraft-legacy` crate. \
            Add `openraft-legacy` to your dependencies and use \
            `openraft_legacy::network_v1::AppendEntriesResponse` instead."
)]
pub struct AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    _p: PhantomData<C>,
}
