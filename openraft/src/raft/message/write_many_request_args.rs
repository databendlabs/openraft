use crate::RaftTypeConfig;
use crate::core::raft_msg::RaftMsg;
use crate::errors::Fatal;
#[cfg(feature = "runtime-stats")]
use crate::raft::api::app::propose_at_now;
use crate::raft::message::WriteOption;
use crate::raft::raft_inner::RaftInner;
use crate::raft::responder::core_responder::CoreResponder;
use crate::type_config::alias::BatchOf;
use crate::type_config::alias::PayloadOf;

/// Arguments of a [`WriteManyRequest`](crate::raft::WriteManyRequest), consumed by
/// [`Self::send`] when the request is awaited.
pub(in crate::raft) struct WriteManyRequestArgs<'a, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) inner: &'a RaftInner<C>,

    /// The payloads to append, in order; `responders[i]` receives the result of `payloads[i]`.
    pub(in crate::raft) payloads: BatchOf<C, PayloadOf<C>>,
    pub(in crate::raft) responders: BatchOf<C, Option<CoreResponder<C>>>,

    pub(in crate::raft) write_option: WriteOption<C>,
}

impl<C> WriteManyRequestArgs<'_, C>
where C: RaftTypeConfig
{
    /// Send all writes to RaftCore in one `ClientWrite` message, so they are admitted together.
    pub(in crate::raft) async fn send(self) -> Result<(), Fatal<C>> {
        self.inner
            .send_msg(RaftMsg::ClientWrite {
                payloads: self.payloads,
                responders: self.responders,
                write_option: self.write_option,
                #[cfg(feature = "runtime-stats")]
                proposed_at: propose_at_now::<C>(),
            })
            .await
    }
}
