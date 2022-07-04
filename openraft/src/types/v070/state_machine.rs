use super::LogId;

/// The changes of a state machine.
/// E.g. when applying a log to state machine, or installing a state machine from snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateMachineChanges {
    pub last_applied: LogId,
    pub is_snapshot: bool,
}
