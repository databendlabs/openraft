use crate::RaftTypeConfig;
use crate::async_runtime::MpscReceiver;
use crate::async_runtime::TryRecvError;
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
}

impl<C, SM> MpscReceiver<Command<C, SM>> for CommandReceiver<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    async fn recv(&mut self) -> Option<Command<C, SM>> {
        match self {
            Self::Bounded(receiver) => receiver.recv().await,
            Self::Buffered(receiver) => receiver.recv().await,
        }
    }

    fn try_recv(&mut self) -> Result<Command<C, SM>, TryRecvError> {
        match self {
            Self::Bounded(receiver) => receiver.try_recv(),
            Self::Buffered(receiver) => receiver.try_recv(),
        }
    }
}
