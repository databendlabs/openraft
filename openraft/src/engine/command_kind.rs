#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
pub(crate) enum CommandKind {
    Log,
    Network,
    StateMachine,
    Other,
}
