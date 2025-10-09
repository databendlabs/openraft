use crate::RaftTypeConfig;
use crate::impls::OneshotResponder;
use crate::raft::ClientWriteResult;
use crate::raft::responder::Responder;
use crate::type_config::alias::ResponderOf;

/// Either an oneshot responder or a user-defined responder.
///
/// It is used in RaftCore to enqueue responder to the client.
pub(crate) enum OneshotOrUserDefined<C>
where C: RaftTypeConfig
{
    Oneshot(OneshotResponder<C>),
    UserDefined(ResponderOf<C>),
}

impl<C> Responder<ClientWriteResult<C>> for OneshotOrUserDefined<C>
where C: RaftTypeConfig
{
    fn send(self, res: ClientWriteResult<C>) {
        match self {
            Self::Oneshot(responder) => responder.send(res),
            Self::UserDefined(responder) => responder.send(res),
        }
    }
}
