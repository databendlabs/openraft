use std::ops::Bound;

use anyerror::AnyError;
use serde::Deserialize;
use serde::Serialize;

use super::HardState;
use super::LogId;
use super::SnapshotMeta;

/// An error that occurs when the RaftStore impl runs defensive check of input or output.
/// E.g. re-applying an log entry is a violation that may be a potential bug.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefensiveError {
    /// The subject that violates store defensive check, e.g. hard-state, log or state machine.
    pub subject: ErrorSubject,

    /// The description of the violation.
    pub violation: Violation,

    pub backtrace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorSubject {
    /// A general storage error
    Store,

    /// HardState related error.
    HardState,

    /// Error that is happened when operating a series of log entries
    Logs,

    /// Error about a single log entry
    Log(LogId),

    /// Error about a single log entry without knowing the log term.
    LogIndex(u64),

    /// Error happened when applying a log entry
    Apply(LogId),

    /// Error happened when operating state machine.
    StateMachine,

    /// Error happened when operating snapshot.
    Snapshot(SnapshotMeta),

    None,
}

/// What it is doing when an error occurs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorVerb {
    Read,
    Write,
    Delete,
}

/// Violations a store would return when running defensive check.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum Violation {
    #[error("term can only be change to a greater value, current: {curr}, change to {to}")]
    TermNotAscending { curr: u64, to: u64 },

    #[error("voted_for can not change from Some() to other Some(), current: {curr:?}, change to {to:?}")]
    VotedForChanged { curr: HardState, to: HardState },

    #[error("log at higher index is obsolete: {higher_index_log_id:?} should GT {lower_index_log_id:?}")]
    DirtyLog {
        higher_index_log_id: LogId,
        lower_index_log_id: LogId,
    },

    #[error("try to get log at index {want} but got {got:?}")]
    LogIndexNotFound { want: u64, got: Option<u64> },

    #[error("range is empty: start: {start:?}, end: {end:?}")]
    RangeEmpty { start: Option<u64>, end: Option<u64> },

    #[error("range is not half-open: start: {start:?}, end: {end:?}")]
    RangeNotHalfOpen { start: Bound<u64>, end: Bound<u64> },

    // TODO(xp): rename this to some input related error name.
    #[error("empty log vector")]
    LogsEmpty,

    #[error("all logs are removed. It requires at least one log to track continuity")]
    StoreLogsEmpty,

    #[error("logs are not consecutive, prev: {prev}, next: {next}")]
    LogsNonConsecutive { prev: LogId, next: LogId },

    #[error("invalid next log to apply: prev: {prev}, next: {next}")]
    ApplyNonConsecutive { prev: LogId, next: LogId },
}

/// A storage error could be either a defensive check error or an error occurred when doing the actual io operation.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageError {
    /// An error raised by defensive check.
    #[error(transparent)]
    Defensive {
        #[from]
        #[backtrace]
        source: DefensiveError,
    },

    /// An error raised by io operation.
    #[error(transparent)]
    IO {
        #[from]
        #[backtrace]
        source: StorageIOError,
    },
}

/// Error that occurs when operating the store.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageIOError {
    pub(crate) subject: ErrorSubject,
    pub(crate) verb: ErrorVerb,
    pub(crate) source: AnyError,
    pub(crate) backtrace: String,
}
