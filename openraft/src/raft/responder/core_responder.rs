use crate::LogId;
use crate::RaftTypeConfig;
use crate::impls::ProgressResponder;
use crate::raft::ClientWriteResult;
use crate::raft::responder::Responder;
use crate::type_config::alias::WriteResponderOf;

/// The responder used in RaftCore.
///
/// RaftCore use this responder to send response to the caller.
/// It is either a progress responder or a user-defined responder.
pub(crate) enum CoreResponder<C>
where C: RaftTypeConfig
{
    Progress(ProgressResponder<C, ClientWriteResult<C>>),
    UserDefined(WriteResponderOf<C>),
}

impl<C> Responder<C, ClientWriteResult<C>> for CoreResponder<C>
where C: RaftTypeConfig
{
    fn on_commit(&mut self, log_id: LogId<C>) {
        match self {
            Self::Progress(responder) => responder.on_commit(log_id),
            Self::UserDefined(responder) => responder.on_commit(log_id),
        }
    }

    fn on_complete(self, res: ClientWriteResult<C>) {
        match self {
            Self::Progress(responder) => responder.on_complete(res),
            Self::UserDefined(responder) => responder.on_complete(res),
        }
    }
}
