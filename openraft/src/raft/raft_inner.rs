use std::fmt;
use std::fmt::Debug;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tracing::Level;

use crate::config::RuntimeConfig;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::RaftMsg;
use crate::core::TickHandle;
use crate::error::Fatal;
use crate::error::RaftError;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftServerMetrics;
use crate::raft::core_state::CoreState;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::AsyncRuntime;
use crate::Config;
use crate::OptionalSend;
use crate::RaftMetrics;
use crate::RaftTypeConfig;

/// RaftInner is the internal handle and provides internally used APIs to communicate with
/// `RaftCore`.
pub(in crate::raft) struct RaftInner<C>
where C: RaftTypeConfig
{
    pub(in crate::raft) id: C::NodeId,
    pub(in crate::raft) config: Arc<Config>,
    pub(in crate::raft) runtime_config: Arc<RuntimeConfig>,
    pub(in crate::raft) tick_handle: TickHandle<C>,
    pub(in crate::raft) tx_api: mpsc::UnboundedSender<RaftMsg<C>>,
    pub(in crate::raft) rx_metrics: watch::Receiver<RaftMetrics<C>>,
    pub(in crate::raft) rx_data_metrics: watch::Receiver<RaftDataMetrics<C>>,
    pub(in crate::raft) rx_server_metrics: watch::Receiver<RaftServerMetrics<C>>,

    // TODO(xp): it does not need to be a async mutex.
    #[allow(clippy::type_complexity)]
    pub(in crate::raft) tx_shutdown: Mutex<Option<OneshotSenderOf<C, ()>>>,
    pub(in crate::raft) core_state: Mutex<CoreState<C>>,

    /// The ongoing snapshot transmission.
    pub(in crate::raft) snapshot: Mutex<Option<crate::network::snapshot_transport::Streaming<C>>>,
}

impl<C> RaftInner<C>
where C: RaftTypeConfig
{
    /// Invoke RaftCore by sending a RaftMsg and blocks waiting for response.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn call_core<T, E>(
        &self,
        mes: RaftMsg<C>,
        rx: OneshotReceiverOf<C, Result<T, E>>,
    ) -> Result<T, RaftError<C, E>>
    where
        E: Debug + OptionalSend,
        T: OptionalSend,
    {
        let sum = if tracing::enabled!(Level::DEBUG) {
            Some(mes.to_string())
        } else {
            None
        };

        let send_res = self.tx_api.send(mes);

        if send_res.is_err() {
            let fatal = self.get_core_stopped_error("sending tx to RaftCore", sum).await;
            return Err(RaftError::Fatal(fatal));
        }

        let recv_res = rx.await;
        tracing::debug!("call_core receives result is error: {:?}", recv_res.is_err());

        match recv_res {
            Ok(x) => x.map_err(|e| RaftError::APIError(e)),
            Err(_) => {
                let fatal = self.get_core_stopped_error("receiving rx from RaftCore", sum).await;
                tracing::error!(error = debug(&fatal), "core_call fatal error");
                Err(RaftError::Fatal(fatal))
            }
        }
    }

    /// Send an [`ExternalCommand`] to RaftCore to execute in the `RaftCore` thread.
    ///
    /// It returns at once.
    pub(in crate::raft) async fn send_external_command(
        &self,
        cmd: ExternalCommand<C>,
        cmd_desc: impl fmt::Display + Default,
    ) -> Result<(), Fatal<C>> {
        let send_res = self.tx_api.send(RaftMsg::ExternalCommand { cmd });

        if send_res.is_err() {
            let fatal = self.get_core_stopped_error("sending external command to RaftCore", Some(cmd_desc)).await;
            return Err(fatal);
        }
        Ok(())
    }

    /// Get the error that caused RaftCore to stop.
    pub(in crate::raft) async fn get_core_stopped_error(
        &self,
        when: impl fmt::Display,
        message_summary: Option<impl fmt::Display + Default>,
    ) -> Fatal<C> {
        // Wait for the core task to finish.
        self.join_core_task().await;

        // Retrieve the result.
        let core_res = {
            let state = self.core_state.lock().await;
            if let CoreState::Done(core_task_res) = &*state {
                core_task_res.clone()
            } else {
                unreachable!("RaftCore should have already quit")
            }
        };

        tracing::error!(
            core_result = debug(&core_res),
            "failure {}; message: {}",
            when,
            message_summary.unwrap_or_default()
        );

        match core_res {
            // A normal quit is still an unexpected "stop" to the caller.
            Ok(_) => Fatal::Stopped,
            Err(e) => e,
        }
    }

    /// Wait for `RaftCore` task to finish and record the returned value from the task.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(in crate::raft) async fn join_core_task(&self) {
        let mut state = self.core_state.lock().await;
        match &mut *state {
            CoreState::Running(handle) => {
                let res = handle.await;
                tracing::info!(res = debug(&res), "RaftCore exited");

                let core_task_res = match res {
                    Err(err) => {
                        if C::AsyncRuntime::is_panic(&err) {
                            Err(Fatal::Panicked)
                        } else {
                            Err(Fatal::Stopped)
                        }
                    }
                    Ok(returned_res) => returned_res,
                };

                *state = CoreState::Done(core_task_res);
            }
            CoreState::Done(_) => {
                // RaftCore has already quit, nothing to do
            }
        }
    }
}
