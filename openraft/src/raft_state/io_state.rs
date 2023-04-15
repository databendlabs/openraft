use crate::LeaderId;
use crate::LogId;
use crate::NodeId;

#[derive(Debug, Clone, Copy)]
#[derive(Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct LogIOId<NID: NodeId> {
    pub(crate) leader_id: LeaderId<NID>,
    pub(crate) log_id: Option<LogId<NID>>,
}

#[derive(Debug, Clone, Copy)]
#[derive(Default)]
#[derive(PartialEq, Eq)]
pub(crate) struct IOState<NID: NodeId> {
    /// The last log id that has been flushed to storage.
    pub(crate) flushed: LogIOId<NID>,
    /// The last log id that has been applied to state machine.
    pub(crate) applied: Option<LogId<NID>>,
}

impl<NID: NodeId> IOState<NID> {
    pub(crate) fn new(flushed: LogIOId<NID>, applied: Option<LogId<NID>>) -> Self {
        Self { flushed, applied }
    }
    pub(crate) fn update_applied(&mut self, log_id: Option<LogId<NID>>) {
        // TODO: should we update flushed if applied is newer?
        debug_assert!(
            log_id > self.applied,
            "applied log id should be monotonically increasing: current: {:?}, update: {:?}",
            self.applied,
            log_id
        );

        self.applied = log_id;
    }

    pub(crate) fn applied(&self) -> Option<&LogId<NID>> {
        self.applied.as_ref()
    }
}
