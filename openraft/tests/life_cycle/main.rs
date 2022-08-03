#![cfg_attr(feature = "bt", feature(backtrace))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t20_initialization;
mod t20_shutdown;
mod t30_connect_error;
