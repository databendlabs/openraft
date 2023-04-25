/// Command kind is used to categorize commands.
///
/// Commands of the different kinds can be paralleled.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
pub(crate) enum CommandKind {
    /// Log IO command
    Log,
    /// Network IO command
    Network,
    /// State machine IO command
    StateMachine,
    /// RaftCore main thread command
    Main,
}
