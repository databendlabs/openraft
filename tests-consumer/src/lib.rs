//! An application that depends on Openraft exactly as the getting-started guide says to.
//!
//! The guide states a version, a feature list, a network API, and a runtime.
//! Each claim is compiled here, from outside the root workspace, so a claim
//! that stops holding fails the build instead of a reader's first `cargo build`:
//!
//! - the feature list is sufficient to declare a [`openraft::RaftTypeConfig`], without the extra
//!   features workspace unification would otherwise supply;
//! - `serde` derives satisfy the bounds that feature adds to the types crossing the network;
//! - [`RaftNetworkV2`] is the network trait an application implements;
//! - the default feature set supplies an [`openraft::AsyncRuntime`], so an application gets tokio
//!   without naming a runtime crate.

use std::fmt;

use openraft::RaftTypeConfig;
use openraft::network::RaftNetworkV2;
use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub key: String,
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Set({})", self.key)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub value: Option<String>,
}

openraft::declare_raft_types!(
    pub TypeConfig:
        D = Request,
        R = Response,
        Node = openraft::NodeInfo,
);

/// The runtime an application gets without asking for one.
pub type Runtime = <TypeConfig as RaftTypeConfig>::AsyncRuntime;

/// Accepts exactly the network implementations the guide tells an application to write.
pub fn accepts_network<N>(_network: N)
where N: RaftNetworkV2<TypeConfig> {
}
