use std::backtrace::Backtrace;
use std::fmt::Formatter;
use std::ops::Bound;

use anyerror::AnyError;
use serde::Deserialize;
use serde::Serialize;

use crate::LogId;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;
use crate::Vote;

/// Convert error to StorageError::IO();
pub trait ToStorageResult<C: RaftTypeConfig, T> {
    /// Convert Result<T, E> to Result<T, StorageError::IO(StorageIOError)>
    ///
    /// `f` provides error context for building the StorageIOError.
    fn sto_res<F>(self, f: F) -> Result<T, StorageError<C>>
    where F: FnOnce() -> (ErrorSubject<C>, ErrorVerb);
}

impl<C: RaftTypeConfig, T> ToStorageResult<C, T> for Result<T, std::io::Error> {
    fn sto_res<F>(self, f: F) -> Result<T, StorageError<C>>
    where F: FnOnce() -> (ErrorSubject<C>, ErrorVerb) {
        match self {
            Ok(x) => Ok(x),
            Err(e) => {
                let (subject, verb) = f();
                let io_err = StorageIOError::new(subject, verb, AnyError::new(&e));
                Err(io_err.into())
            }
        }
    }
}

/// An error that occurs when the RaftStore impl runs defensive check of input or output.
/// E.g. re-applying an log entry is a violation that may be a potential bug.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefensiveError<C: RaftTypeConfig> {
    /// The subject that violates store defensive check, e.g. hard-state, log or state machine.
    pub subject: ErrorSubject<C>,

    /// The description of the violation.
    pub violation: Violation<C>,

    pub backtrace: String,
}

impl<C: RaftTypeConfig> DefensiveError<C> {
    pub fn new(subject: ErrorSubject<C>, violation: Violation<C>) -> Self {
        Self {
            subject,
            violation,
            backtrace: format!("{:?}", Backtrace::capture()),
        }
    }
}

impl<C: RaftTypeConfig> std::fmt::Display for DefensiveError<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{:?}' violates: '{}'", self.subject, self.violation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorSubject<C: RaftTypeConfig> {
    /// A general storage error
    Store,

    /// HardState related error.
    Vote,

    /// Error that is happened when operating a series of log entries
    Logs,

    /// Error about a single log entry
    Log(LogId<C>),

    /// Error about a single log entry without knowing the log term.
    LogIndex(u64),

    /// Error happened when applying a log entry
    Apply(LogId<C>),

    /// Error happened when operating state machine.
    StateMachine,

    /// Error happened when operating snapshot.
    Snapshot(SnapshotMeta<C>),

    None,
}

/// What it is doing when an error occurs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorVerb {
    Read,
    Write,
    Seek,
    Delete,
}

/// Violations a store would return when running defensive check.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum Violation<C: RaftTypeConfig> {
    #[error("term can only be change to a greater value, current: {curr}, change to {to}")]
    TermNotAscending { curr: u64, to: u64 },

    #[error("voted_for can not change from Some() to other Some(), current: {curr:?}, change to {to:?}")]
    NonIncrementalVote { curr: Vote<C>, to: Vote<C> },

    #[error("log at higher index is obsolete: {higher_index_log_id:?} should GT {lower_index_log_id:?}")]
    DirtyLog {
        higher_index_log_id: LogId<C>,
        lower_index_log_id: LogId<C>,
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

    #[error("logs are not consecutive, prev: {prev:?}, next: {next}")]
    LogsNonConsecutive { prev: Option<LogId<C>>, next: LogId<C> },

    #[error("invalid next log to apply: prev: {prev:?}, next: {next}")]
    ApplyNonConsecutive { prev: Option<LogId<C>>, next: LogId<C> },

    #[error("applied log can not conflict, last_applied: {last_applied:?}, delete since: {first_conflict_log_id}")]
    AppliedWontConflict {
        last_applied: Option<LogId<C>>,
        first_conflict_log_id: LogId<C>,
    },

    #[error("not allowed to purge non-applied logs, last_applied: {last_applied:?}, purge upto: {purge_upto}")]
    PurgeNonApplied {
        last_applied: Option<LogId<C>>,
        purge_upto: LogId<C>,
    },
}

/// A storage error could be either a defensive check error or an error occurred when doing the actual io operation.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageError<C: RaftTypeConfig> {
    /// An error raised by defensive check.
    #[error(transparent)]
    Defensive {
        #[from]
        #[backtrace]
        source: DefensiveError<C>,
    },

    /// An error raised by io operation.
    #[error(transparent)]
    IO {
        #[from]
        #[backtrace]
        source: StorageIOError<C>,
    },
}

impl<C: RaftTypeConfig> StorageError<C> {
    pub fn into_defensive(self) -> Option<DefensiveError<C>> {
        match self {
            StorageError::Defensive { source } => Some(source),
            _ => None,
        }
    }

    pub fn into_io(self) -> Option<StorageIOError<C>> {
        match self {
            StorageError::IO { source } => Some(source),
            _ => None,
        }
    }

    pub fn from_io_error(subject: ErrorSubject<C>, verb: ErrorVerb, io_error: std::io::Error) -> Self {
        let sto_io_err = StorageIOError::new(subject, verb, AnyError::new(&io_error));
        StorageError::IO { source: sto_io_err }
    }
}

/// Error that occurs when operating the store.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageIOError<C: RaftTypeConfig> {
    subject: ErrorSubject<C>,
    verb: ErrorVerb,
    source: AnyError,
    backtrace: String,
}

impl<C: RaftTypeConfig> std::fmt::Display for StorageIOError<C> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "when {:?} {:?}: {}", self.verb, self.subject, self.source)
    }
}

impl<C: RaftTypeConfig> StorageIOError<C> {
    pub fn new(subject: ErrorSubject<C>, verb: ErrorVerb, source: AnyError) -> Self {
        Self {
            subject,
            verb,
            source,
            // TODO: use crate backtrace instead of std::backtrace.
            backtrace: format!("{:?}", Backtrace::capture()),
        }
    }
}
