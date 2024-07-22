use std::fmt;

use anyerror::AnyError;

use crate::storage::SnapshotSignature;
use crate::LogId;
use crate::RaftTypeConfig;

/// Convert error to StorageError::IO();
pub trait ToStorageResult<C, T>
where C: RaftTypeConfig
{
    /// Convert `Result<T, E>` to `Result<T, StorageError>`
    ///
    /// `f` provides error context for building the StorageError.
    fn sto_res<F>(self, f: F) -> Result<T, StorageError<C>>
    where F: FnOnce() -> (ErrorSubject<C>, ErrorVerb);
}

impl<C, T> ToStorageResult<C, T> for Result<T, std::io::Error>
where C: RaftTypeConfig
{
    fn sto_res<F>(self, f: F) -> Result<T, StorageError<C>>
    where F: FnOnce() -> (ErrorSubject<C>, ErrorVerb) {
        match self {
            Ok(x) => Ok(x),
            Err(e) => {
                let (subject, verb) = f();
                let io_err = StorageError::new(subject, verb, AnyError::new(&e));
                Err(io_err)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ErrorSubject<C>
where C: RaftTypeConfig
{
    /// A general storage error
    Store,

    /// HardState related error.
    Vote,

    /// Error that is happened when operating a series of log entries
    Logs,

    /// Error about a single log entry
    Log(LogId<C::NodeId>),

    /// Error about a single log entry without knowing the log term.
    LogIndex(u64),

    /// Error happened when applying a log entry
    Apply(LogId<C::NodeId>),

    /// Error happened when operating state machine.
    StateMachine,

    /// Error happened when operating snapshot.
    Snapshot(Option<SnapshotSignature<C>>),

    None,
}

/// What it is doing when an error occurs.
#[derive(Debug)]
#[derive(Clone, Copy)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ErrorVerb {
    Read,
    Write,
    Seek,
    Delete,
}

impl fmt::Display for ErrorVerb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Backward compatible with old application using `StorageIOError`
#[deprecated(note = "use StorageError instead", since = "0.10.0")]
pub type StorageIOError<C> = StorageError<C>;

impl<C> StorageError<C>
where C: RaftTypeConfig
{
    /// Backward compatible with old form `StorageError::IO{ source: StorageError }`
    #[deprecated(note = "no need to call this method", since = "0.10.0")]
    pub fn into_io(self) -> Option<StorageError<C>> {
        Some(self)
    }

    pub fn from_io_error(subject: ErrorSubject<C>, verb: ErrorVerb, io_error: std::io::Error) -> Self {
        StorageError::new(subject, verb, AnyError::new(&io_error))
    }
}

/// Error that occurs when operating the store.
///
/// It indicates a data crash.
/// An application returning this error will shutdown the Openraft node immediately to prevent
/// further damage.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct StorageError<C>
where C: RaftTypeConfig
{
    subject: ErrorSubject<C>,
    verb: ErrorVerb,
    source: AnyError,
    backtrace: Option<String>,
}

impl<C> fmt::Display for StorageError<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "when {:?} {:?}: {}", self.verb, self.subject, self.source)
    }
}

impl<C> StorageError<C>
where C: RaftTypeConfig
{
    pub fn new(subject: ErrorSubject<C>, verb: ErrorVerb, source: impl Into<AnyError>) -> Self {
        Self {
            subject,
            verb,
            source: source.into(),
            backtrace: anyerror::backtrace_str(),
        }
    }

    pub fn write_log_entry(log_id: LogId<C::NodeId>, source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Log(log_id), ErrorVerb::Write, source)
    }

    pub fn read_log_at_index(log_index: u64, source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::LogIndex(log_index), ErrorVerb::Read, source)
    }

    pub fn read_log_entry(log_id: LogId<C::NodeId>, source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Log(log_id), ErrorVerb::Read, source)
    }

    pub fn write_logs(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Logs, ErrorVerb::Write, source)
    }

    pub fn read_logs(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Logs, ErrorVerb::Read, source)
    }

    pub fn write_vote(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Vote, ErrorVerb::Write, source)
    }

    pub fn read_vote(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Vote, ErrorVerb::Read, source)
    }

    pub fn apply(log_id: LogId<C::NodeId>, source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Apply(log_id), ErrorVerb::Write, source)
    }

    pub fn write_state_machine(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::StateMachine, ErrorVerb::Write, source)
    }

    pub fn read_state_machine(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::StateMachine, ErrorVerb::Read, source)
    }

    pub fn write_snapshot(signature: Option<SnapshotSignature<C>>, source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Snapshot(signature), ErrorVerb::Write, source)
    }

    pub fn read_snapshot(signature: Option<SnapshotSignature<C>>, source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Snapshot(signature), ErrorVerb::Read, source)
    }

    /// General read error
    pub fn read(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Store, ErrorVerb::Read, source)
    }

    /// General write error
    pub fn write(source: impl Into<AnyError>) -> Self {
        Self::new(ErrorSubject::Store, ErrorVerb::Write, source)
    }
}
