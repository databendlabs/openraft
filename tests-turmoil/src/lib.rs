//! Turmoil-based simulation tests for OpenRaft.
//!
//! This crate provides deterministic simulation testing for the Raft consensus algorithm
//! using the turmoil framework. It allows testing network partitions, message delays,
//! and other failure scenarios in a reproducible manner.

// Determinism depends on the forked turmoil seeding tokio's runtime RNG
// (`Builder::rng_seed`, a `tokio_unstable` API): without it, `tokio::sync::watch`
// wakes waiters in random order. The flag comes from this crate's
// `.cargo/config.toml`, which cargo only reads when invoked from inside this
// directory, and whose rustflags any `RUSTFLAGS` environment variable
// overrides — so fail loudly instead of silently losing reproducibility.
#[cfg(not(tokio_unstable))]
compile_error!(
    "tests-turmoil requires `--cfg tokio_unstable`: build from the tests-turmoil directory \
     so cargo reads tests-turmoil/.cargo/config.toml, and keep the RUSTFLAGS environment \
     variable unset — it overrides the config's rustflags"
);

pub mod cluster;
pub mod invariants;
pub mod liveness;
pub mod network;
pub mod oracle;
#[cfg(test)]
mod scenarios;
pub mod store;
pub mod typ;

pub use typ::*;
