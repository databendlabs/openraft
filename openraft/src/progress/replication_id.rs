use std::fmt;
use std::fmt::Formatter;
use std::ops::Deref;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplicationId {
    id: u64,
}

impl ReplicationId {
    pub(crate) fn new(id: u64) -> Self {
        Self { id }
    }
}

impl Deref for ReplicationId {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl fmt::Display for ReplicationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}
