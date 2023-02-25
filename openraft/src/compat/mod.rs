//! This mod is a upgrade helper that provides functionalities for a newer openraft application to
//! read data written by an older application.
//!
//! This mod is enabled by feature flag `compat`.

#[cfg(feature = "compat-07")] pub mod compat07;
pub mod testing;

mod upgrade;

pub use upgrade::Compat;
pub use upgrade::Upgrade;
