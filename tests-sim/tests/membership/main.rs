//! openraft's membership integration tests on the deterministic simulated runtime (openraft#206).
//!
//! The fixtures and test files are openraft's own, included by path rather than copied. The
//! memstore `TypeConfig` runs on `openraft_rt_sim::SimRuntime` through its `rt-sim` feature, and
//! this crate patches in the futures-util fork, as tests-turmoil does, so rt-sim reseeds the
//! `select!` shuffle RNG at the start of every run.
//!
//! `t24_append_membership.rs` is included three times, as `run1`, `run2` and `run3`, so one
//! process can run the same scenario back to back:
//!
//! ```text
//! cargo test --test membership -- --test-threads=1 follower_answers_forward_to_leader
//! ```

#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![allow(clippy::uninlined_format_args)]
// Only the t24 scenarios use the fixtures here, so most helpers go unused.
#![allow(dead_code)]
// `run1`, `run2` and `run3` include the same file on purpose; see above.
#![allow(clippy::duplicate_mod)]

#[macro_use]
#[path = "../../../tests/tests/fixtures/mod.rs"]
mod fixtures;

#[path = "../../../tests/tests/membership/t24_append_membership.rs"]
mod run1;
#[path = "../../../tests/tests/membership/t24_append_membership.rs"]
mod run2;
#[path = "../../../tests/tests/membership/t24_append_membership.rs"]
mod run3;
