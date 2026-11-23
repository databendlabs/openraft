use std::fmt;

use serde::Deserialize;
use serde::Serialize;

/// Identifies one write of one client, so that a state machine applies the write at most once.
///
/// A client numbers its writes with increasing serials and sends them one at a time: it may resend
/// its latest write, with the same ID, any number of times, but never an earlier one. A state
/// machine then needs to remember only the latest write of each client. It does not apply a write
/// whose serial is not above the latest one it applied for that client, so a restarted client
/// takes a new `client_id` instead of counting again from the start.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId {
    /// The client that sends the write, unique among all clients and their restarts, such as a
    /// random number.
    pub client_id: u64,

    /// The position of the write among the writes of its client. A resent write keeps it.
    pub serial: u64,
}

impl RequestId {
    pub fn new(client_id: u64, serial: u64) -> Self {
        RequestId { client_id, serial }
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.client_id, self.serial)
    }
}
