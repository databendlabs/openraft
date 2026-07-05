/// Entry stored in [`VecProgress`].
///
/// `VecProgress` only uses the ID and progress value. Other entry fields are
/// application-owned state.
///
/// [`VecProgress`]: crate::progress::VecProgress
pub(crate) trait VecProgressEntry {
    type Id: 'static + PartialEq;
    type Progress: Clone + Default + Ord;

    /// Return the ID this entry belongs to.
    fn id(&self) -> &Self::Id;

    /// Return the monotonic progress value used to calculate quorum acceptance.
    fn progress(&self) -> &Self::Progress;

    /// Return the mutable monotonic progress value.
    fn progress_mut(&mut self) -> &mut Self::Progress;

    /// Return the ID and progress value as references.
    fn id_progress(&self) -> (&Self::Id, &Self::Progress) {
        (self.id(), self.progress())
    }

    /// Return cloned ID and progress value.
    fn id_progress_owned(&self) -> (Self::Id, Self::Progress)
    where Self::Id: Clone {
        let (id, progress) = self.id_progress();
        (id.clone(), progress.clone())
    }
}

/// Entry with application-owned data stored beside progress.
#[allow(dead_code)]
pub(crate) trait VecProgressEntryData: VecProgressEntry {
    type Data;

    /// Return the application-owned data.
    fn data(&self) -> &Self::Data;

    /// Return mutable application-owned data.
    fn data_mut(&mut self) -> &mut Self::Data;
}

impl<ID, Progress> VecProgressEntry for (ID, Progress)
where
    ID: 'static + PartialEq,
    Progress: Clone + Default + Ord,
{
    type Id = ID;
    type Progress = Progress;

    fn id(&self) -> &Self::Id {
        &self.0
    }

    fn progress(&self) -> &Self::Progress {
        &self.1
    }

    fn progress_mut(&mut self) -> &mut Self::Progress {
        &mut self.1
    }
}
