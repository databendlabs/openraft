use std::fmt;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;

use crate::config::RuntimeConfig;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::core::raft_msg::RaftMsg;
use crate::core::TickHandle;
use crate::error::Fatal;
use crate::raft::core_state::CoreState;
use crate::AsyncRuntime;
use crate::Config;
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
    pub(in crate::raft) rx_metrics: watch::Receiver<RaftMetrics<C::NodeId, C::Node>>,

    // TODO(xp): it does not need to be a async mutex.
    #[allow(clippy::type_complexity)]
    pub(in crate::raft) tx_shutdown: Mutex<Option<oneshot::Sender<()>>>,
    pub(in crate::raft) core_state: Mutex<CoreState<C::NodeId, C::AsyncRuntime>>,
}

impl<C> RaftInner<C>
where C: RaftTypeConfig
{
    /// Send an [`ExternalCommand`] to RaftCore to execute in the `RaftCore` thread.
    ///
    /// It returns at once.
    pub(in crate::raft) async fn send_external_command(
        &self,
        cmd: ExternalCommand,
        cmd_desc: impl fmt::Display + Default,
    ) -> Result<(), Fatal<C::NodeId>> {
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
    ) -> Fatal<C::NodeId> {
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
