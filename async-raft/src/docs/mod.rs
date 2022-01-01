//! ### log and state machine
//!
//! ```
//!
//! ------------+------+------+-----+------+-----+----> log index
//!             |      |      |     |      |     ` last_log
//!             |      |      |     |      ` committed
//!             |      |      |     ` last_applied
//!             |      |      ` snapshot
//!             |      ` purged
//!             ` delayed follower
//!
//! ```
//!  TODO: explain each concept related to log positions in the above diagram.
