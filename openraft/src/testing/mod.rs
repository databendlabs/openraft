//! Testing utilities for Openraft applications.
//!
//! This module provides test utilities and suite runners to verify Openraft implementations.
//!
//! ## Modules
//!
//! - [`common`] - Common test utilities and assertions
//! - [`log`] - Log storage test suite
//! - [`runtime`] - Runtime test utilities
//!
//! ## Overview
//!
//! Test suites help verify that custom implementations of storage and network traits
//! behave correctly according to Raft protocol requirements.
//!
//! ## Usage
//!
//! Import test utilities to verify your implementations:
//!
//! ```ignore
//! use openraft::testing::log::Suite;
//!
//! #[test]
//! fn test_log_storage() {
//!     Suite::test_all(MyLogStore::new());
//! }
//! ```
//!
//! These tests help ensure correctness and catch subtle protocol violations.

pub mod common;
pub mod log;
pub mod runtime;

pub use common::*;
