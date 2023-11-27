#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

mod t10_build_snapshot;
mod t35_building_snapshot_does_not_block_append;
mod t35_building_snapshot_does_not_block_apply;
mod t60_snapshot_policy_never;
