//! This mod is an upgrade helper that provides functionalities for a newer openraft application to
//! read data written by an older application.

mod upgrade;

pub use upgrade::Compat;
pub use upgrade::Upgrade;
