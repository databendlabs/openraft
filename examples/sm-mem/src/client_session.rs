use openraft::macros::since;
use serde::Deserialize;
use serde::Serialize;

/// What the state machine remembers of one client that tags its writes with a
/// [`types_kv::RequestId`]: the client's latest applied write, and the response it produced.
///
/// One entry per client is enough because a client sends its writes one at a time and never
/// resends an earlier one; see [`types_kv::RequestId`].
#[since(version = "0.10.2")]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    /// The serial of the client's latest applied write.
    pub serial: u64,

    /// The response that write produced when it was applied, returned again to each resent copy.
    pub response: types_kv::Response,
}
