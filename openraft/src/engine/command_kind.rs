/// Command kind is used to categorize commands.
///
/// Commands of different kinds can be parallelized.
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
    /// Command handled by RaftCore main thread.
    Main,
    /// Respond to caller. Can be executed in parallel with other commands.
    Respond,
}
