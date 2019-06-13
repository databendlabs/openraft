//! A module holding definitions of all the Actix message types used in this crate.

use actix::prelude::*;

use crate::proto;

/// An actix::Message wrapping a protobuf RaftRequest.
pub struct RaftRequest(pub proto::RaftRequest);

impl Message for RaftRequest {
    type Result = Result<proto::RaftResponse, ()>;
}
