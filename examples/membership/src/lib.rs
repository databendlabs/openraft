//! Run each membership that OpenRaft provides in an in-process cluster.
//!
//! [`Policy`] names the six memberships: the default majority `Membership` of `openraft` and the
//! five memberships of `openraft-membership`. [`run()`] runs the scenario of one of them. The CLI
//! in `src/main.rs` and the tests in `tests/` call the same function.

#![deny(unused_qualifications)]

pub mod scenarios;

pub use scenarios::Policy;
pub use scenarios::run;
