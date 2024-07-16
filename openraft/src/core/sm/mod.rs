//! State machine worker and its supporting types.
//!
//! This worker runs in a separate task and is the only one that can mutate the state machine.
//! It is responsible for applying log entries, building/receiving snapshot  and sending responses
//! to the RaftCore.

pub(crate) mod command;
pub(crate) mod handle;
pub(crate) mod response;
pub(crate) mod worker;

pub(crate) use command::Command;
pub(crate) use response::CommandResult;
pub(crate) use response::Response;
