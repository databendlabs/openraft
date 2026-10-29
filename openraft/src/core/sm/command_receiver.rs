use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::mpsc::BufferedReceiver;
use crate::core::sm::Command;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::MpscReceiverOf;

pub(crate) enum CommandReceiver<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    Bounded(MpscReceiverOf<C, Command<C, SM>>),
    Buffered(BufferedReceiver<C::AsyncRuntime, Command<C, SM>>),
}

impl<C, SM> CommandReceiver<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(receiver: MpscReceiverOf<C, Command<C, SM>>, buffered: bool) -> Self {
        if buffered {
            let receiver = receiver.with_unbounded_buffer::<C::AsyncRuntime>();
            return Self::Buffered(receiver);
        }
        Self::Bounded(receiver)
    }
    pub(crate) async fn recv(&mut self) -> Option<Command<C, SM>> {
        match self {
            Self::Bounded(receiver) => receiver.recv().await,
            Self::Buffered(receiver) => receiver.recv().await,
        }
    }
}
