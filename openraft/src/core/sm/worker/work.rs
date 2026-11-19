use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::mpsc::PeekableReceiver;
use crate::core::sm::Command;
use crate::core::sm::apply_batch::ApplyBatch;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::OneshotSenderOf;

/// FIFO work shared by the serial reader and applier stages.
pub(super) enum Work<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    Apply(ApplyBatch<C>),
    Barrier {
        command: Command<C, SM>,
        finished: OneshotSenderOf<C, ()>,
    },
}

impl<C, SM> Work<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Merge only a contiguous prefix that is already visible in this stage's channel.
    pub(super) fn coalesce<R>(&mut self, incoming: &mut PeekableReceiver<R, Self>)
    where R: MpscReceiver<Self> {
        let Self::Apply(batch) = self else { return };
        while let Ok(Self::Apply(next)) = incoming.try_peek() {
            if next.since() != batch.end() {
                break;
            }
            let Ok(Self::Apply(next)) = incoming.try_recv() else {
                unreachable!("the peeked batch remains in the receiver");
            };
            batch.extend(next);
        }
    }
}
