//! Legacy compatibility layer for Openraft.
//!
//! This crate provides backward compatibility for applications using deprecated
//! Openraft APIs. Instead of modifying application code, users can switch imports
//! from the main `openraft` crate to this crate.
//!
//! # Available Legacy APIs
//!
//! ## Network V1
//!
//! The [`network_v1`] module provides the v1 `RaftNetwork` trait with chunk-based
//! snapshot transmission. See [`network_v1`] module documentation for usage.
//!
//! ```ignore
//! // Old import:
//! // use openraft::network::RaftNetwork;
//!
//! // New import for legacy API:
//! use openraft_legacy::network_v1::RaftNetwork;
//! ```

pub mod network_v1;
