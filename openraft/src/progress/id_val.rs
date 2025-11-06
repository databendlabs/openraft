use std::fmt;

use crate::LogId;
use crate::progress::entry;

/// An ID and its associated value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdVal<ID, Val> {
    pub(crate) id: ID,
    pub(crate) val: Val,
}

impl<ID, Val> fmt::Display for IdVal<ID, Val>
where
    ID: fmt::Display,
    Val: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.id, self.val)
    }
}

impl<ID, Val> IdVal<ID, Val> {
    pub(crate) fn new(id: ID, val: Val) -> Self {
        Self { id, val }
    }
}

impl<ID, C> IdVal<ID, entry::ProgressEntry<C>>
where
    C: crate::RaftTypeConfig,
    ID: Clone,
{
    /// Return a tuple of (cloned id, cloned matching log id).
    pub(crate) fn to_matching_tuple(&self) -> (ID, Option<LogId<C>>) {
        (self.id.clone(), self.val.matching().cloned())
    }
}
