use std::fmt::Formatter;

use anyerror::AnyError;

use crate::types::v065::DefensiveError;
use crate::types::v065::ErrorSubject;
use crate::types::v065::Violation;
use crate::ErrorVerb;
use crate::StorageError;
use crate::StorageIOError;

impl DefensiveError {
    pub fn new(subject: ErrorSubject, violation: Violation) -> DefensiveError {
        DefensiveError {
            subject,
            violation,
            backtrace: anyerror::backtrace_str(),
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
            backtrace: anyerror::backtrace_str(),
        }
    }
}
