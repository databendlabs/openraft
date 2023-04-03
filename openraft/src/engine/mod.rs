//! Engine is an abstracted, complete raft consensus algorithm implementation.
//!
//! The internal `state` inside `Engine` contains complete information to run raft, i.e., it is a in
//! memory shadow of the underlying storage state.
//! Every time `Engine` receives an event, such as handle-vote-request, or elect, it updates its
//! internal state and emits several `Command` to let the underlying `Runtime` to take place the
//! state changes, e.g. `AppendInputEntries` or `LeaderCommit`.
//! I.e., `Runtime` is an adaptor connecting raft algorithm and concrete storage/network
//! implementation.
//!
//! ```text
//!                      Runtime         Engine
//!                      |               |
//!  Application ------> + ------------> +      // event: app-write, change-membership
//!                      |               |
//!  Timer ------------> + ------------> +      // event: election, heartbeat
//!                      |               |
//!  Storage <---------- + <------------ +      // cmd: append, truncate, purge, commit
//!                      |               |
//!       .------------- + <------------ +      // cmd: replicate-log, vote, install-snapshot
//!       v              |               |
//!  Network ----------> + ------------> +      // event: replication result, vote result
//!                      |               |
//!
//!
//!  ------->: event to Engine
//!  <-------: command to run
//! ```

mod command;
mod engine_config;
mod engine_impl;
mod engine_output;
mod log_id_list;

pub(crate) mod handler;
pub(crate) mod time_state;

#[cfg(test)] mod command_test;
#[cfg(test)] mod elect_test;
#[cfg(test)] mod handle_append_entries_req_test;
#[cfg(test)] mod handle_vote_req_test;
#[cfg(test)] mod handle_vote_resp_test;
#[cfg(test)] mod initialize_test;
#[cfg(test)] mod log_id_list_test;
#[cfg(test)] mod startup_test;
#[cfg(test)] mod testing;
#[cfg(test)] mod update_progress_test;

pub(crate) use command::Command;
pub(crate) use command::SendResult;
pub(crate) use engine_config::EngineConfig;
pub(crate) use engine_impl::Engine;
pub(crate) use engine_output::EngineOutput;
pub use log_id_list::LogIdList;

use crate::RaftTypeConfig;

/// A type alias that use `C: RaftTypeConfig` as generic parameter and is used internally with a
/// shorter name for convenience.
#[allow(dead_code)]
pub(crate) type CEngine<C> =
    Engine<<C as RaftTypeConfig>::NodeId, <C as RaftTypeConfig>::Node, <C as RaftTypeConfig>::Entry>;
