#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]
#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

// Used by bin/bench.rs
use clap as _;
use libc as _;
use maplit as _;

pub mod network;
pub mod store;

pub mod profile;
pub mod profile_request;
pub mod profile_value;
mod wire_append;
mod wire_link;

mod byte_rate;
