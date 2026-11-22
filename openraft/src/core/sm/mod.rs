//! State machine worker and its supporting types.
//!
//! After startup, the worker runs in a task of its own and owns the state machine: it applies log
//! entries, installs snapshots, and runs external callbacks on it, one command at a time. Recovery
//! before startup applies committed logs and installs a snapshot without the worker.
//!
//! The worker installs a snapshot only once the transport has received all of its data; receiving
//! snapshot data is not a worker command. To build a snapshot, it spawns a separate task, so the
//! build may overlap the commands that follow.
//!
//! It tells RaftCore through notifications when it has applied entries, installed a snapshot, or
//! finished or deferred a build. Other commands, such as reading the current snapshot, answer
//! their caller directly.

pub(crate) mod apply_batch;
pub(crate) mod command;
pub(crate) mod command_receiver;
pub(crate) mod handle;
pub(crate) mod response;
pub(crate) mod worker;

pub(crate) use command::Command;
pub(crate) use response::CommandResult;
pub(crate) use response::Response;
