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
            backtrace: anyerror::backtrace_str(),
        }
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
}

impl StorageIOError {
    pub fn new(subject: ErrorSubject, verb: ErrorVerb, source: AnyError) -> StorageIOError {
        StorageIOError {
            subject,
            verb,
            source,
            backtrace: anyerror::backtrace_str(),
        }
    }
}
