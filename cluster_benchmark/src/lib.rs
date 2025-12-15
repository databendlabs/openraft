#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]
#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

// Used by bin/bench.rs
use clap as _;
use maplit as _;

pub mod network;
pub mod store;
