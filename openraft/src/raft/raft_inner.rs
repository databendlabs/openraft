use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tracing::Level;

use crate::Config;
use crate::OptionalSend;
use crate::RaftMetrics;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscSender;
use crate::async_runtime::watch::WatchReceiver;
use crate::async_runtime::watch::WatchSender;
use crate::config::RuntimeConfig;
use crate::core::TickHandle;
use crate::core::io_flush_tracking::IoProgressWatcher;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::display_ext::DisplayOptionExt;
use crate::error::Fatal;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::Wait;
use crate::raft::core_state::CoreState;
use crate::type_config::AsyncRuntime;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::MutexOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::WatchReceiverOf;

/// RaftInner is the internal handle and provides internally used APIs to communicate with
/// `RaftCore`.
pub(in crate::raft) struct RaftInner<C>
where C: RaftTypeConfig
{
    pub(in crate::raft) id: C::NodeId,
    pub(in crate::raft) config: Arc<Config>,
    pub(in crate::raft) runtime_config: Arc<RuntimeConfig>,
    pub(in crate::raft) tick_handle: TickHandle<C>,
    pub(in crate::raft) tx_api: MpscSenderOf<C, RaftMsg<C>>,
    pub(in crate::raft) rx_metrics: WatchReceiverOf<C, RaftMetrics<C>>,
    pub(in crate::raft) rx_data_metrics: WatchReceiverOf<C, RaftDataMetrics<C>>,
    pub(in crate::raft) rx_server_metrics: WatchReceiverOf<C, RaftServerMetrics<C>>,
    pub(in crate::raft) progress_watcher: IoProgressWatcher<C>,

    pub(in crate::raft) tx_shutdown: std::sync::Mutex<Option<OneshotSenderOf<C, ()>>>,
    pub(in crate::raft) core_state: std::sync::Mutex<CoreState<C>>,

    /// The ongoing snapshot transmission.
    #[cfg_attr(not(feature = "tokio-rt"), allow(dead_code))]
    // This field will only be read when feature tokio-rt is on
    pub(in crate::raft) snapshot: MutexOf<C, Option<crate::network::snapshot_transport::Streaming<C>>>,
}

impl<C> RaftInner<C>
where C: RaftTypeConfig
{
    pub(crate) fn id(&self) -> &C::NodeId {
        &self.id
    }

    pub(crate) fn config(&self) -> &Config {
        self.config.as_ref()
    }

    pub(crate) async fn send_msg(&self, mes: RaftMsg<C>) -> Result<(), Fatal<C>> {
        let send_res = self.tx_api.send(mes).await;

        if let Err(e) = send_res {
            let msg = e.0;

            let fatal = self.get_core_stop_error().await;
            tracing::error!("Failed to send RaftMsg: {msg} to RaftCore; error: {fatal}",);
            return Err(fatal);
        }
        Ok(())
    }

    /// Invoke RaftCore by sending a RaftMsg and blocks waiting for response.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn call_core<T>(&self, mes: RaftMsg<C>, rx: OneshotReceiverOf<C, T>) -> Result<T, Fatal<C>>
    where T: OptionalSend {
        let sum = if tracing::enabled!(Level::DEBUG) {
            Some(mes.to_string())
        } else {
            None
        };

        self.send_msg(mes).await?;
        self.recv_msg(rx).await.inspect_err(|_e| {
            tracing::error!("Failed to receive from RaftCore: error when {}", sum.display());
        })
    }

    /// Receive a message from RaftCore, return an error if RaftCore has stopped.
    pub(crate) async fn recv_msg<T, E>(&self, rx: impl Future<Output = Result<T, E>>) -> Result<T, Fatal<C>>
    where
        T: OptionalSend,
        E: OptionalSend,
    {
        let recv_res = rx.await;
        tracing::debug!("{} receives result is error: {:?}", func_name!(), recv_res.is_err());

        match recv_res {
            Ok(x) => Ok(x),
            Err(_) => {
                let fatal = self.get_core_stop_error().await;
                tracing::error!(error = debug(&fatal), "error when {}", func_name!());
                Err(fatal)
            }
        }
    }

    /// Send an [`ExternalCommand`] to RaftCore to execute in the `RaftCore` thread.
    ///
    /// It returns at once.
    pub(in crate::raft) async fn send_external_command(&self, cmd: ExternalCommand<C>) -> Result<(), Fatal<C>> {
        let send_res = self.tx_api.send(RaftMsg::ExternalCommand { cmd }).await;

        if send_res.is_err() {
            let fatal = self.get_core_stop_error().await;
            return Err(fatal);
        }
        Ok(())
    }

    pub(in crate::raft) fn is_core_running(&self) -> bool {
        let state = self.core_state.lock().unwrap();
        state.is_running()
    }

    /// Get the error that caused RaftCore to stop.
    pub(crate) async fn get_core_stop_error(&self) -> Fatal<C> {
        // Wait for the core task to finish.
        self.join_core_task().await;

        // Retrieve the result.
        let core_res = {
            let state = self.core_state.lock().unwrap();
            if let CoreState::Done(core_task_res) = &*state {
                core_task_res.clone()
            } else {
                unreachable!("RaftCore should have already quit")
            }
        };

        // Safe unwrap: core_res is always an error
        core_res.unwrap_err()
    }

    /// Wait for `RaftCore` task to finish and record the returned value from the task.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(in crate::raft) async fn join_core_task(&self) {
        // Get the Running state of RaftCore,
        // or an error if RaftCore has been in Joining state.
        let running_res = {
            let mut state = self.core_state.lock().unwrap();

            match &*state {
                CoreState::Running(_) => {
                    let (tx, rx) = C::watch_channel::<bool>(false);

                    let prev = std::mem::replace(&mut *state, CoreState::Joining(rx));

                    let CoreState::Running(join_handle) = prev else {
                        unreachable!()
                    };

                    Ok((join_handle, tx))
                }
                CoreState::Joining(watch_rx) => Err(watch_rx.clone()),
                CoreState::Done(_) => {
                    // RaftCore has already finished exiting, nothing to do
                    return;
                }
            }
        };

        match running_res {
            Ok((join_handle, tx)) => {
                let join_res = join_handle.await;

                tracing::info!(res = debug(&join_res), "RaftCore exited");

                let core_task_res = match join_res {
                    Err(err) => {
                        if AsyncRuntimeOf::<C>::is_panic(&err) {
                            Err(Fatal::Panicked)
                        } else {
                            Err(Fatal::Stopped)
                        }
                    }
                    Ok(returned_res) => returned_res,
                };

                {
                    let mut state = self.core_state.lock().unwrap();
                    *state = CoreState::Done(core_task_res);
                }
                tx.send(true).ok();
            }
            Err(mut rx) => {
                // Another thread is waiting for the core to finish.
                loop {
                    let res = rx.changed().await;
                    if res.is_err() {
                        break;
                    }
                    if *rx.borrow_watched() {
                        break;
                    }
                }
            }
        }
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>) -> Wait<C> {
        let timeout = timeout.unwrap_or_else(|| Duration::from_secs(86400 * 365 * 100));

        Wait {
            timeout,
            rx: self.rx_metrics.clone(),
        }
    }
}
