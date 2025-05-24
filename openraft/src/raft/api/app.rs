use std::error::Error;
use std::future::Future;

use openraft_macros::since;

use crate::core::raft_msg::RaftMsg;
use crate::error::CheckIsLeaderError;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::raft_inner::RaftInner;
use crate::raft::responder::Responder;
use crate::raft::ClientWriteResponse;
use crate::raft::ClientWriteResult;
use crate::type_config::alias::ResponderOf;
use crate::type_config::alias::ResponderReceiverOf;
use crate::type_config::TypeConfigExt;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::ReadPolicy;

/// Provides application-facing APIs for interacting with the Raft system.
///
/// This struct contains methods for client operations such as linearizable reads
/// and writes.
#[since(version = "0.10.0")]
pub(crate) struct AppApi<'a, C>
where C: RaftTypeConfig
{
    inner: &'a RaftInner<C>,
}

impl<'a, C> AppApi<'a, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) fn new(inner: &'a RaftInner<C>) -> Self {
        Self { inner }
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn get_read_linearizer(
        &self,
        read_policy: ReadPolicy,
    ) -> Result<Result<Linearizer<C>, CheckIsLeaderError<C>>, Fatal<C>> {
        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::CheckIsLeaderRequest { read_policy, tx }, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, app_data))]
    pub(crate) async fn client_write<E>(
        &self,
        app_data: C::D,
        // TODO: ClientWriteError can only be ForwardToLeader Error
    ) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, Fatal<C>>
    where
        ResponderReceiverOf<C>: Future<Output = Result<ClientWriteResult<C>, E>>,
        E: Error + OptionalSend,
    {
        let rx = self.client_write_ff(app_data).await?;

        let res: ClientWriteResult<C> = self.inner.recv_msg(rx).await?;

        Ok(res)
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, app_data))]
    pub(crate) async fn client_write_ff(&self, app_data: C::D) -> Result<ResponderReceiverOf<C>, Fatal<C>> {
        let (app_data, tx, rx) = ResponderOf::<C>::from_app_data(app_data);

        self.inner.send_msg(RaftMsg::ClientWriteRequest { app_data, tx }).await?;

        Ok(rx)
    }
}
