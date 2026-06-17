use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use display_more::DisplayOptionExt;
use tracing::Level;

use crate::Config;
use crate::Extensions;
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
use crate::errors::Fatal;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::Wait;
use crate::raft::core_state::CoreState;
use crate::type_config::AsyncRuntime;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::AsyncRuntimeOf;
use crate::type_config::alias::MpscSenderOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;
use crate::type_config::alias::WatchReceiverOf;

/// How long [`RaftInner::recv_msg`] waits for RaftCore to actually stop after a response channel
/// closed, before concluding the channel was closed by a dropped responder instead.
///
/// A real stop is observable almost immediately -- RaftCore reports it in metrics and drops the
/// metrics sender as its task ends -- so this only needs to tolerate scheduling latency, not an
/// actual shutdown. It solely bounds how long a caller waits on the dropped-responder path.
const RECV_CORE_STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// RaftInner is the internal handle and provides internally used APIs to communicate with
/// `RaftCore`.
pub(in crate::raft) struct RaftInner<C, SD = ()>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    pub(in crate::raft) id: C::NodeId,
    pub(in crate::raft) config: Arc<Config>,
    pub(in crate::raft) runtime_config: Arc<RuntimeConfig>,
    pub(in crate::raft) tick_handle: TickHandle<C>,
    pub(in crate::raft) tx_api: MpscSenderOf<C, RaftMsg<C, SD>>,
    pub(in crate::raft) rx_metrics: WatchReceiverOf<C, RaftMetrics<C>>,
    pub(in crate::raft) rx_data_metrics: WatchReceiverOf<C, RaftDataMetrics<C>>,
    pub(in crate::raft) rx_server_metrics: WatchReceiverOf<C, RaftServerMetrics<C>>,
    pub(in crate::raft) progress_watcher: IoProgressWatcher<C>,

    pub(in crate::raft) tx_shutdown: Mutex<Option<OneshotSenderOf<C, ()>>>,
    pub(in crate::raft) core_state: Mutex<CoreState<C>>,

    /// Type-map for storing user-defined extension data.
    ///
    /// External crates can access this via [`Raft::extensions()`](`crate::Raft::extensions`).
    pub(in crate::raft) extensions: Extensions,
}

impl<C, SD> RaftInner<C, SD>
where
    C: RaftTypeConfig,
    SD: OptionalSend + 'static,
{
    pub(crate) fn id(&self) -> &C::NodeId {
        &self.id
    }

    pub(crate) fn config(&self) -> &Config {
        self.config.as_ref()
    }

    pub(crate) async fn send_msg(&self, mes: RaftMsg<C, SD>) -> Result<(), Fatal<C>> {
        let send_res = self.tx_api.send(mes).await;

        if let Err(e) = send_res {
            let msg = e.0;

            let fatal = self.get_core_stop_error().await;
            tracing::error!("failed to send RaftMsg to RaftCore: {msg}, error: {fatal}",);
            return Err(fatal);
        }
        Ok(())
    }

    /// Invoke RaftCore by sending a RaftMsg and blocks waiting for response.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn call_core<T>(&self, mes: RaftMsg<C, SD>, rx: OneshotReceiverOf<C, T>) -> Result<T, Fatal<C>>
    where T: OptionalSend {
        let sum = if tracing::enabled!(Level::DEBUG) {
            Some(mes.to_string())
        } else {
            None
        };

        self.send_msg(mes).await?;
        self.recv_msg(rx).await.inspect_err(|_e| {
            tracing::error!("failed to receive from RaftCore: {}", sum.display());
        })
    }

    /// Receive a message from RaftCore, return an error if the response is not delivered.
    pub(crate) async fn recv_msg<T, E>(&self, rx: impl Future<Output = Result<T, E>>) -> Result<T, Fatal<C>>
    where
        T: OptionalSend,
        E: OptionalSend,
    {
        let recv_res = rx.await;
        tracing::debug!("{}: receives result is error: {:?}", func_name!(), recv_res.is_err());

        match recv_res {
            Ok(x) => Ok(x),
            Err(_) => {
                // The response channel was closed without a reply. Usually this means RaftCore has
                // stopped, and we return the fatal error that stopped it. But a malfunctioning
                // state machine can drop the responder without replying while RaftCore is still
                // running; joining the core then would block forever. Bound the wait: if the core
                // has not stopped, the dropped responder is itself the error, and we return at once
                // instead of waiting any longer.
                let fatal = if self.wait_core_stopped(RECV_CORE_STOP_TIMEOUT).await {
                    let fatal = self.get_core_stop_error().await;
                    tracing::error!("{}: error: {}", func_name!(), fatal);
                    fatal
                } else {
                    tracing::error!(
                        "{}: response dropped without a reply while RaftCore is running",
                        func_name!()
                    );
                    Fatal::Stopped
                };

                Err(fatal)
            }
        }
    }

    /// Wait up to `timeout` for RaftCore to stop, without joining (and thus consuming) it.
    ///
    /// Observes the metrics watch channel, a non-destructive signal safe to poll from any number of
    /// callers: RaftCore sets [`running_state`](RaftMetrics::running_state) to `Err` when it stops,
    /// and drops the metrics sender when its task ends -- including on panic. Either is a reliable
    /// signal that the core has stopped.
    ///
    /// Returns `true` if a stop was observed, `false` if the core is still running after `timeout`
    /// (e.g. the response channel was closed by a dropped responder rather than a core shutdown).
    async fn wait_core_stopped(&self, timeout: Duration) -> bool {
        let mut rx = self.rx_metrics.clone();
        let observe = async move {
            loop {
                if rx.borrow_watched().running_state.is_err() {
                    return;
                }
                // `changed()` errors only when the metrics sender is dropped, i.e. the core task
                // has ended.
                if rx.changed().await.is_err() {
                    return;
                }
            }
        };
        C::timeout(timeout, observe).await.is_ok()
    }

    /// Send an [`ExternalCommand`] to RaftCore to execute in the `RaftCore` thread.
    ///
    /// It returns at once.
    pub(in crate::raft) async fn send_external_command(&self, cmd: ExternalCommand<C, SD>) -> Result<(), Fatal<C>> {
        let send_res = self.tx_api.send(RaftMsg::ExternalCommand { cmd }).await;

        if send_res.is_err() {
            let fatal = self.get_core_stop_error().await;
            return Err(fatal);
        }
        Ok(())
    }

    #[allow(dead_code)]
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

                tracing::info!("RaftCore exited: {:?}", join_res);

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
