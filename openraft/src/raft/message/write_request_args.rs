use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::core::raft_msg::RaftMsg;
use crate::entry::RaftPayload;
use crate::errors::Fatal;
#[cfg(feature = "runtime-stats")]
use crate::raft::api::app::propose_at_now;
use crate::raft::message::WriteOption;
use crate::raft::raft_inner::RaftInner;
use crate::raft::responder::core_responder::CoreResponder;

/// Arguments of a [`WriteRequest`](crate::raft::WriteRequest), consumed by [`Self::send`] when
/// the request is awaited.
pub(in crate::raft) struct WriteRequestArgs<'a, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) inner: &'a RaftInner<C>,
    pub(in crate::raft) app_data: C::D,
    pub(in crate::raft) responder: Option<CoreResponder<C>>,
    pub(in crate::raft) write_option: WriteOption<C>,
}

impl<C> WriteRequestArgs<'_, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) async fn send(self) -> Result<(), Fatal<C>> {
        self.inner
            .send_msg(RaftMsg::ClientWrite {
                payloads: Batch::of([C::Payload::normal(self.app_data)]),
                responders: Batch::of([self.responder]),
                write_option: self.write_option,
                #[cfg(feature = "runtime-stats")]
                proposed_at: propose_at_now::<C>(),
            })
            .await
    }
}
