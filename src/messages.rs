//! A module holding definitions of all the Actix message types used in this crate.

use actix::prelude::*;

use crate::proto;

/// An actix::Message wrapping a protobuf RaftMessage.
pub struct RaftMessage(pub proto::RaftMessage);

impl Message for RaftMessage {
    type Result = Result<proto::RaftMessage, ()>;
}
