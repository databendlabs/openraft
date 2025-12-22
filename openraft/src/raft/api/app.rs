use std::sync::Arc;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::ReadPolicy;
use crate::base::Batch;
use crate::base::BoxStream;
use crate::core::raft_msg::RaftMsg;
use crate::error::ClientWriteError;
use crate::error::Fatal;
use crate::error::LinearizableReadError;
use crate::impls::ProgressResponder;
use crate::raft::ClientWriteResponse;
use crate::raft::ClientWriteResult;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::message::WriteResult;
use crate::raft::message::into_write_result;
use crate::raft::raft_inner::RaftInner;
use crate::raft::responder::core_responder::CoreResponder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::WriteResponderOf;

/// Provides application-facing APIs for interacting with the Raft system.
///
/// This struct contains methods for client operations such as linearizable reads
/// and writes.
#[since(version = "0.10.0")]
pub(crate) struct AppApi<'a, C>
where C: RaftTypeConfig
{
    inner: &'a Arc<RaftInner<C>>,
}

impl<'a, C> AppApi<'a, C>
where C: RaftTypeConfig
{
    pub(in crate::raft) fn new(inner: &'a Arc<RaftInner<C>>) -> Self {
        Self { inner }
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn get_read_linearizer(
        &self,
        read_policy: ReadPolicy,
    ) -> Result<Result<Linearizer<C>, LinearizableReadError<C>>, Fatal<C>> {
        let (tx, rx) = C::oneshot();
        self.inner.call_core(RaftMsg::GetLinearizer { read_policy, tx }, rx).await
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self, app_data))]
    pub(crate) async fn client_write(
        &self,
        app_data: C::D,
        // TODO: ClientWriteError can only be ForwardToLeader Error
    ) -> Result<Result<ClientWriteResponse<C>, ClientWriteError<C>>, Fatal<C>> {
        let (responder, _commit_rx, complete_rx) = ProgressResponder::new();

        self.do_client_write_ff(
            Batch::from(app_data),
            Batch::from(Some(CoreResponder::Progress(responder))),
        )
        .await?;

        let res: ClientWriteResult<C> = self.inner.recv_msg(complete_rx).await?;

        Ok(res)
    }

    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn client_write_ff(
        &self,
        app_data: C::D,
        responder: Option<WriteResponderOf<C>>,
    ) -> Result<(), Fatal<C>> {
        self.do_client_write_ff(
            Batch::from(app_data),
            Batch::from(responder.map(|r| CoreResponder::UserDefined(r))),
        )
        .await
    }

    /// Fire-and-forget version of `client_write`, accept a generic responder.
    #[since(version = "0.10.0")]
    async fn do_client_write_ff(
        &self,
        app_data: Batch<C::D>,
        responders: Batch<Option<CoreResponder<C>>>,
    ) -> Result<(), Fatal<C>> {
        self.inner
            .send_msg(RaftMsg::ClientWrite {
                app_data,
                responders,
                expected_leader: None,
            })
            .await?;

        Ok(())
    }

    /// Write multiple application data payloads in a single batch.
    ///
    /// Returns a stream that yields each result in submission order.
    /// This is more efficient than calling [`client_write()`](Self::client_write) multiple times
    /// as it sends all payloads in a single message to the Raft core.
    ///
    /// If RaftCore stops, the stream yields `Err(Fatal::Stopped)` and ends.
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn client_write_many(
        &self,
        app_data: impl IntoIterator<Item = C::D>,
    ) -> Result<BoxStream<'static, Result<WriteResult<C>, Fatal<C>>>, Fatal<C>> {
        let app_data_vec: Vec<C::D> = app_data.into_iter().collect();

        let mut responders = Vec::with_capacity(app_data_vec.len());
        let mut receivers = Vec::with_capacity(app_data_vec.len());

        for _ in 0..app_data_vec.len() {
            let (responder, _commit_rx, complete_rx) = ProgressResponder::<C, ClientWriteResult<C>>::new();
            responders.push(Some(CoreResponder::Progress(responder)));
            receivers.push(complete_rx);
        }

        self.do_client_write_ff(Batch::from(app_data_vec), Batch::from(responders)).await?;

        let stream = futures::stream::unfold(Some(receivers.into_iter()), |opt_iter| async move {
            let mut iter = opt_iter?;
            let rx = iter.next()?;
            match rx.await {
                Ok(result) => Some((Ok(into_write_result(result)), Some(iter))),
                Err(_) => Some((Err(Fatal::Stopped), None)),
            }
        });

        Ok(Box::pin(stream))
    }
}
