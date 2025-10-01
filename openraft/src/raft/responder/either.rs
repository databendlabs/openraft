use crate::RaftTypeConfig;
use crate::impls::OneshotResponder;
use crate::raft::ClientWriteResult;
use crate::raft::responder::Responder;

/// Either an oneshot responder or a user-defined responder.
///
/// It is used in RaftCore to enqueue responder to the client.
pub(crate) enum OneshotOrUserDefined<C>
where C: RaftTypeConfig
{
    Oneshot(OneshotResponder<C>),
    UserDefined(C::Responder),
}

impl<C> Responder<C> for OneshotOrUserDefined<C>
where C: RaftTypeConfig
{
    fn send(self, res: ClientWriteResult<C>) {
        match self {
            Self::Oneshot(responder) => responder.send(res),
            Self::UserDefined(responder) => responder.send(res),
        }
    }

    type Receiver = ();

    fn from_app_data(_app_data: <C as RaftTypeConfig>::D) -> (<C as RaftTypeConfig>::D, Self, Self::Receiver)
    where Self: Sized {
        unimplemented!("OneshotOrUserDefined is just a wrapper and does not support building from app_data")
    }
}
