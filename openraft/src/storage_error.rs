use std::backtrace::Backtrace;
use std::fmt::Formatter;

use anyerror::AnyError;

use crate::DefensiveError;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::StorageError;
use crate::StorageIOError;
use crate::Violation;

impl DefensiveError {
    pub fn new(subject: ErrorSubject, violation: Violation) -> DefensiveError {
        DefensiveError {
            subject,
            violation,
            backtrace: format!("{:?}", Backtrace::capture()),
        }
    }
}

impl std::fmt::Display for DefensiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{:?}' violates: '{}'", self.subject, self.violation)
    }
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

    pub fn from_io_error(subject: ErrorSubject, verb: ErrorVerb, io_error: std::io::Error) -> Self {
        let sto_io_err = StorageIOError::new(subject, verb, AnyError::new(&io_error));
        StorageError::IO { source: sto_io_err }
    }
}

impl std::fmt::Display for StorageIOError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "when {:?} {:?}: {}", self.verb, self.subject, self.source)
    }
}

impl StorageIOError {
    pub fn new(subject: ErrorSubject, verb: ErrorVerb, source: AnyError) -> StorageIOError {
        StorageIOError {
            subject,
            verb,
            source,
            // TODO: use crate backtrace instead of std::backtrace.
            backtrace: format!("{:?}", Backtrace::capture()),
        }
    }
}
