use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct InvalidStateMachineType {
    pub actual_type: &'static str,
}

impl fmt::Display for InvalidStateMachineType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "User-defined function on the state machine fails to run; It may have used different type of state machine from the one in RaftCore",
        )
    }
}

impl InvalidStateMachineType {
    pub(crate) fn new<SM>() -> Self {
        Self {
            actual_type: std::any::type_name::<SM>(),
        }
    }
}
