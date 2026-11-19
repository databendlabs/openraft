use crate::RaftTypeConfig;
use crate::core::sm::Command;
use crate::core::sm::apply_batch::ApplyBatch;
use crate::storage::RaftStateMachine;

pub(super) enum Job<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    Read(ApplyBatch<C>),
    Apply(ApplyBatch<C>),
    Command(Command<C, SM>),
}
