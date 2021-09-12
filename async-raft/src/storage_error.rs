use std::backtrace::Backtrace;
use std::fmt::Formatter;
use std::ops::Bound;

use crate::storage::HardState;
use crate::LogId;
use crate::SnapshotMeta;

/// An error that occurs when the RaftStore impl runs defensive check of input or output.
/// E.g. re-applying an log entry is a violation that may be a potential bug.
#[derive(thiserror::Error, Debug)]
pub struct DefensiveError {
    /// The subject that violates store defensive check, e.g. hard-state, log or state machine.
    pub subject: ErrorSubject,

    /// The description of the violation.
    pub violation: Violation,

    pub backtrace: Backtrace,
}

impl DefensiveError {
    pub fn new(subject: ErrorSubject, violation: Violation) -> DefensiveError {
        DefensiveError {
            subject,
            violation,
            backtrace: Backtrace::capture(),
        }
    }
}

impl std::fmt::Display for DefensiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{:?}' violates: '{}'", self.subject, self.violation)
    }
}

#[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug)]
pub enum ErrorVerb {
    Read,
    Write,
    Delete,
}

/// Violations a store would return when running defensive check.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
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

    #[error("empty log vector")]
    LogsEmpty,

    #[error("logs are not consecutive, prev: {prev}, next: {next}")]
    LogsNonConsecutive { prev: LogId, next: LogId },

    #[error("invalid next log to apply: prev: {prev}, next: {next}")]
    ApplyNonConsecutive { prev: LogId, next: LogId },
}

/// A storage error could be either a defensive check error or an error occurred when doing the actual io operation.
#[derive(Debug, thiserror::Error)]
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

impl StorageError {
    pub fn into_defensive(self) -> Option<DefensiveError> {
        match self {
            StorageError::Defensive { source } => Some(source),
            _ => None,
        }
    }

    pub fn into_io(self) -> Option<StorageIOError> {
        match self {
            StorageError::IO { source } => Some(source),
            _ => None,
        }
    }
}

/// Error that occurs when operating the store.
#[derive(Debug, thiserror::Error)]
pub struct StorageIOError {
    subject: ErrorSubject,
    verb: ErrorVerb,
    source: anyhow::Error,
    backtrace: Backtrace,
}

impl std::fmt::Display for StorageIOError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "when {:?} {:?}: {}", self.verb, self.subject, self.source)
    }
}

impl StorageIOError {
    pub fn new(subject: ErrorSubject, verb: ErrorVerb, source: anyhow::Error) -> StorageIOError {
        StorageIOError {
            subject,
            verb,
            source,
            backtrace: Backtrace::capture(),
        }
    }
}
