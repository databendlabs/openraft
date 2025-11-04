use crate::Membership;
use crate::RaftTypeConfig;
use crate::raft::responder::core_responder::CoreResponder;
use crate::type_config::alias::LogIdOf;

/// Internal implementation of [`ApplyResponder`](super::ApplyResponder).
pub(crate) enum ApplyResponderInner<C: RaftTypeConfig> {
    /// Normal entry response without membership change.
    Normal {
        log_id: LogIdOf<C>,
        responder: CoreResponder<C>,
    },

    /// Entry response with membership change.
    Membership {
        log_id: LogIdOf<C>,
        membership: Membership<C>,
        responder: CoreResponder<C>,
    },
}

impl<C: RaftTypeConfig> ApplyResponderInner<C> {
    /// Send the response after applying an entry.
    pub(crate) fn send(self, response: C::R) {
        use crate::raft::ClientWriteResponse;
        use crate::raft::responder::Responder as ResponderTrait;

        match self {
            ApplyResponderInner::Normal { log_id, responder } => {
                let res = Ok(ClientWriteResponse {
                    log_id,
                    data: response,
                    membership: None,
                });
                responder.on_complete(res);
            }
            ApplyResponderInner::Membership {
                log_id,
                membership,
                responder,
            } => {
                let res = Ok(ClientWriteResponse {
                    log_id,
                    data: response,
                    membership: Some(membership),
                });
                responder.on_complete(res);
            }
        }
    }
}
