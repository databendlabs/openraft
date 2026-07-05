use std::fmt;

use crate::progress::VecProgressEntry;
#[cfg(test)]
use crate::progress::VecProgressEntryData;

/// An ID and its associated value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdVal<ID, Val> {
    pub(crate) id: ID,
    pub(crate) val: Val,
}

/// An ID, progress, and application-owned data.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdValData<ID, Val, Data> {
    pub(crate) id: ID,
    pub(crate) val: Val,
    pub(crate) data: Data,
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

#[cfg(test)]
impl<ID, Val, Data> fmt::Display for IdValData<ID, Val, Data>
where
    ID: fmt::Display,
    Val: fmt::Display,
    Data: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.id, self.val, self.data)
    }
}

impl<ID, Val> IdVal<ID, Val> {
    pub(crate) fn new(id: ID, val: Val) -> Self {
        Self { id, val }
    }
}

impl<ID, Val> IdVal<ID, Val>
where Val: Default
{
    /// Create an [`IdVal`] with the provided ID and `Val::default()`.
    pub(crate) fn new_default(id: ID) -> Self {
        Self::new(id, Default::default())
    }
}

#[cfg(test)]
impl<ID, Val, Data> IdValData<ID, Val, Data> {
    pub(crate) fn new(id: ID, val: Val, data: Data) -> Self {
        Self { id, val, data }
    }
}

impl<ID, Val> VecProgressEntry for IdVal<ID, Val>
where
    ID: 'static + PartialEq,
    Val: Clone + Default + Ord,
{
    type Id = ID;
    type Progress = Val;

    fn id(&self) -> &Self::Id {
        &self.id
    }

    fn progress(&self) -> &Self::Progress {
        &self.val
    }

    fn progress_mut(&mut self) -> &mut Self::Progress {
        &mut self.val
    }
}

#[cfg(test)]
impl<ID, Val, Data> VecProgressEntry for IdValData<ID, Val, Data>
where
    ID: 'static + PartialEq,
    Val: Clone + Default + Ord,
{
    type Id = ID;
    type Progress = Val;

    fn id(&self) -> &Self::Id {
        &self.id
    }

    fn progress(&self) -> &Self::Progress {
        &self.val
    }

    fn progress_mut(&mut self) -> &mut Self::Progress {
        &mut self.val
    }
}

#[cfg(test)]
impl<ID, Val, Data> VecProgressEntryData for IdValData<ID, Val, Data>
where
    ID: 'static + PartialEq,
    Val: Clone + Default + Ord,
{
    type Data = Data;

    fn data(&self) -> &Self::Data {
        &self.data
    }

    fn data_mut(&mut self) -> &mut Self::Data {
        &mut self.data
    }
}
