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
use crate::config::RuntimeConfig;
use crate::core::TickHandle;
use crate::core::io_flush_tracking::IoProgressWatcher;
use crate::core::raft_msg::RaftMsg;
use crate::core::raft_msg::external_command::ExternalCommand;
use crate::errors::Fatal;
use crate::errors::Infallible;
use crate::metrics::RaftDataMetrics;
use crate::metrics::RaftServerMetrics;
use crate::metrics::Wait;
use crate::raft::core_state::CoreState;
use crate::type_config::TypeConfigExt;
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
pub(in crate::raft) struct RaftInner<C>
where C: RaftTypeConfig
{
    pub(in crate::raft) id: C::NodeId,
    pub(in crate::raft) config: Arc<Config>,
    pub(in crate::raft) runtime_config: Arc<RuntimeConfig<C>>,
    pub(in crate::raft) tick_handle: Mutex<Option<TickHandle<C>>>,
    pub(in crate::raft) tx_api: MpscSenderOf<C, RaftMsg<C>>,
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
            tracing::error!("failed to send RaftMsg to RaftCore: {msg}, error: {fatal}",);
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
            tracing::error!("failed to receive from RaftCore: {}", sum.display());
        })
    }

    /// Invoke RaftCore with a message that carries a oneshot responder, and block waiting for the
    /// response.
    ///
    /// The sender half is handed to `make_msg` and the matching receiver half is awaited here.
    /// Prefer this over [`Self::call_core`]: `RaftMsg` does not carry the response type, so
    /// `call_core` accepts any receiver, including one belonging to a different request.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn call_core_oneshot<T, F>(&self, make_msg: F) -> Result<T, Fatal<C>>
    where
        T: OptionalSend,
        F: FnOnce(OneshotSenderOf<C, T>) -> RaftMsg<C>,
    {
        let (tx, rx) = C::oneshot();
        self.call_core(make_msg(tx), rx).await
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
    pub(in crate::raft) async fn send_external_command(&self, cmd: ExternalCommand<C>) -> Result<(), Fatal<C>> {
        let send_res = self.tx_api.send(RaftMsg::ExternalCommand { cmd }).await;

        if send_res.is_err() {
            let fatal = self.get_core_stop_error().await;
            return Err(fatal);
        }
        Ok(())
    }

    /// Get the error that caused RaftCore to stop.
    pub(crate) async fn get_core_stop_error(&self) -> Fatal<C> {
        let (core_res, _) = self.join_core_task().await;

        // Safe unwrap: core_res is always an error
        core_res.unwrap_err()
    }

    /// Like [`Self::get_core_stop_error`], but does not block forever when the caller can't tell
    /// whether RaftCore itself stopped or a *different*, independently-failing task did.
    ///
    /// Waits up to [`RECV_CORE_STOP_TIMEOUT`] for RaftCore to stop; if it hasn't by then, RaftCore
    /// is still running and [`Self::get_core_stop_error`] would block on it indefinitely, so this
    /// returns `Fatal::Stopped` instead.
    pub(crate) async fn get_core_stop_error_bounded(&self) -> Fatal<C> {
        if self.wait_core_stopped(RECV_CORE_STOP_TIMEOUT).await {
            self.get_core_stop_error().await
        } else {
            Fatal::Stopped
        }
    }

    /// Wait for `RaftCore` and return its result and whether its runtime join failed.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(in crate::raft) async fn join_core_task(&self) -> (Result<Infallible, Fatal<C>>, bool) {
        let join = {
            let mut state = self.core_state.lock().unwrap();
            match &*state {
                CoreState::Unstarted(_) | CoreState::StoppedBeforeRun => {
                    // Dropping the unstarted future closes all of RaftCore's channels.
                    *state = CoreState::StoppedBeforeRun;
                    return (Err(Fatal::Stopped), false);
                }
                CoreState::Started(join) => join.clone(),
            }
        };
        join.await
    }

    pub(crate) fn wait(&self, timeout: Option<Duration>) -> Wait<C> {
        Wait {
            timeout,
            rx: self.rx_metrics.clone(),
        }
    }
}

#[cfg(test)]
mod raft_inner_test;
