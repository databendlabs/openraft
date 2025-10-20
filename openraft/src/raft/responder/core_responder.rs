use crate::RaftTypeConfig;
use crate::impls::OneshotResponder;
use crate::raft::ClientWriteResult;
use crate::raft::responder::Responder;
use crate::type_config::alias::WriteResponderOf;

/// The responder used in RaftCore.
///
/// RaftCore use this responder to send response to the caller.
/// It is either an oneshot responder or a user-defined responder.
pub(crate) enum CoreResponder<C>
where C: RaftTypeConfig
{
    Oneshot(OneshotResponder<C>),
    UserDefined(WriteResponderOf<C>),
}

impl<C> Responder<ClientWriteResult<C>> for CoreResponder<C>
where C: RaftTypeConfig
{
    fn send(self, res: ClientWriteResult<C>) {
        match self {
            Self::Oneshot(responder) => responder.send(res),
            Self::UserDefined(responder) => responder.send(res),
        }
    }
}
