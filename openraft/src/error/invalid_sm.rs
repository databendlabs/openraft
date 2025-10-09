/// Error indicating that a state machine operation was attempted with an incompatible type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "User-defined function on the state machine failed to run; \
     It may have used a different type \
     of state machine from the one in RaftCore (`{actual_type}`)"
)]
pub struct InvalidStateMachineType {
    /// The actual type name of the state machine that was used.
    pub actual_type: &'static str,
}

impl InvalidStateMachineType {
    pub(crate) fn new<SM>() -> Self {
        Self {
            actual_type: std::any::type_name::<SM>(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_invalid_state_machine_type_to_string() {
        let err = super::InvalidStateMachineType::new::<u32>();
        assert_eq!(
            err.to_string(),
            "User-defined function on the state machine failed to run; It may have used a different type of state machine from the one in RaftCore (`u32`)"
        );
    }
}
