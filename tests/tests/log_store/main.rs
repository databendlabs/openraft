#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![allow(clippy::uninlined_format_args)]
#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_save_committed;
