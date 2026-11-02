use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::FutureExt;

use super::RaftInner;
use crate::Config;
use crate::Extensions;
use crate::Raft;
use crate::RaftMetrics;
use crate::async_runtime::MpscSender;
use crate::async_runtime::OneshotSender;
use crate::config::RuntimeConfig;
use crate::core::TickHandle;
use crate::core::io_flush_tracking::IoProgressWatcher;
use crate::errors::Fatal;
use crate::errors::Infallible;
use crate::errors::ShutdownError;
use crate::metrics::RaftServerMetrics;
use crate::raft::core_state::CoreState;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;

crate::declare_raft_types!(UTConfig: AsyncRuntime = openraft_rt_tokio::TokioRuntime);

fn raft_with_tasks(
    core: JoinHandleOf<UTConfig, Result<Infallible, Fatal<UTConfig>>>,
    tick: JoinHandleOf<UTConfig, ()>,
) -> Raft<UTConfig> {
    let config = Arc::new(Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    });
    let (tx_shutdown, _rx_shutdown) = UTConfig::oneshot();
    let (tick_shutdown, _tick_shutdown_rx) = UTConfig::oneshot();
    let inner = RaftInner {
        id: 0,
        runtime_config: Arc::new(RuntimeConfig::new(&config)),
        config,
        tick_handle: Mutex::new(Some(TickHandle::new(tick_shutdown, tick))),
        tx_api: UTConfig::mpsc(1).0,
        rx_metrics: UTConfig::watch_channel(RaftMetrics::new_initial(0)).1,
        rx_data_metrics: UTConfig::watch_channel(Default::default()).1,
        rx_server_metrics: UTConfig::watch_channel(RaftServerMetrics::new_initial(0)).1,
        progress_watcher: IoProgressWatcher::new().1,
        tx_shutdown: Mutex::new(Some(tx_shutdown)),
        core_state: Mutex::new(CoreState::started(core)),
        extensions: Extensions::default(),
    };
    Raft {
        inner: Arc::new(inner),
        sm_cmd_tx: UTConfig::mpsc(1).0.downgrade(),
        install_snapshot_tx: UTConfig::mpsc(1).0,
    }
}

#[test]
fn try_shutdown_core_panic_after_internal_join() {
    UTConfig::run(async {
        let (tick_tx, tick_rx) = UTConfig::oneshot();
        let core = UTConfig::spawn(async { panic!("injected core task panic") });
        let tick = UTConfig::spawn(async move { tick_rx.await.unwrap() });
        let raft = raft_with_tasks(core, tick);

        tracing::info!("retain the core panic even when a request joins it before shutdown");
        {
            let fatal = raft.inner.get_core_stop_error().await;
            assert_eq!(Fatal::Panicked, fatal);
            let pending = raft.try_shutdown().now_or_never();
            assert!(pending.is_none());
        }

        tracing::info!("finish joining tick before returning the retained core join failure");
        {
            tick_tx.send(()).unwrap();
            let result = UTConfig::timeout(Duration::from_secs(1), raft.try_shutdown()).await.unwrap();
            let expected = Err(ShutdownError);
            assert_eq!(expected, result);
            let repeated = raft.try_shutdown().await;
            assert_eq!(result, repeated);
        }
    });
}

#[test]
fn try_shutdown_tick_panic() {
    UTConfig::run(async {
        tracing::info!("surface a tick task panic and retain it across shutdown calls");
        {
            let raft = raft_with_tasks(
                UTConfig::spawn(async { Err(Fatal::Stopped) }),
                UTConfig::spawn(async { panic!("injected tick task panic") }),
            );
            let result = raft.try_shutdown().await;
            let expected = Err(ShutdownError);
            assert_eq!(expected, result);
            let repeated = raft.try_shutdown().await;
            assert_eq!(result, repeated);
        }
    });
}

#[test]
fn try_shutdown_aborted_tasks() {
    UTConfig::run(async {
        tracing::info!("abort both tasks so shutdown must retain a join failure");
        {
            let core = UTConfig::spawn(std::future::pending());
            let tick = UTConfig::spawn(std::future::pending());
            core.abort();
            tick.abort();
            let raft = raft_with_tasks(core, tick);
            let result = raft.try_shutdown().await;
            let expected = Err(ShutdownError);
            assert_eq!(expected, result);
            let repeated = raft.try_shutdown().await;
            assert_eq!(result, repeated);
        }
    });
}

#[test]
fn try_shutdown_resumes_cancelled_join() {
    UTConfig::run(async {
        let (core_tx, core_rx) = UTConfig::oneshot();
        let (tick_tx, tick_rx) = UTConfig::oneshot();
        let raft = raft_with_tasks(
            UTConfig::spawn(async move {
                core_rx.await.unwrap();
                Err(Fatal::Stopped)
            }),
            UTConfig::spawn(async move { tick_rx.await.unwrap() }),
        );

        tracing::info!("drop a shutdown waiting on the core; its join must remain available");
        {
            let pending = raft.try_shutdown().now_or_never();
            assert!(pending.is_none());
            let mut first = Box::pin(raft.inner.join_core_task());
            let pending = first.as_mut().now_or_never();
            assert!(pending.is_none());
            let mut second = Box::pin(raft.inner.join_core_task());
            let pending = second.as_mut().now_or_never();
            assert!(pending.is_none());
            core_tx.send(()).unwrap();
            let (first, second) = UTConfig::timeout(Duration::from_secs(1), async {
                futures_util::future::join(first, second).await
            })
            .await
            .unwrap();
            let expected = (Err(Fatal::Stopped), false);
            assert_eq!(expected, first);
            assert_eq!(expected, second);
        }

        tracing::info!("drop a shutdown waiting on the tick; subsequent callers share that join");
        {
            let pending = raft.try_shutdown().now_or_never();
            assert!(pending.is_none());
            tick_tx.send(()).unwrap();
            let (first, second) = UTConfig::timeout(Duration::from_secs(1), async {
                futures_util::future::join(raft.try_shutdown(), raft.try_shutdown()).await
            })
            .await
            .unwrap();
            let expected = Ok(());
            assert_eq!(expected, first);
            assert_eq!(first, second);
        }
    });
}

#[test]
fn shutdown_retry_waits_for_core_after_cancelled_shutdown() {
    UTConfig::run(async {
        let (core_tx, core_rx) = UTConfig::oneshot();
        let raft = raft_with_tasks(
            UTConfig::spawn(async move {
                core_rx.await.unwrap();
                Err(Fatal::Stopped)
            }),
            UTConfig::spawn(async {}),
        );

        tracing::info!("cancel shutdown while the core is blocked, then retry the public API");
        let retry = {
            let pending = raft.shutdown().now_or_never();
            assert!(pending.is_none());
            let mut retry = Box::pin(raft.shutdown());
            let pending = retry.as_mut().now_or_never();
            assert!(pending.is_none());
            retry
        };

        tracing::info!("release the core so the retried shutdown can finish and retain its result");
        {
            core_tx.send(()).unwrap();
            UTConfig::timeout(Duration::from_secs(1), retry).await.unwrap().unwrap();
            let fatal = raft.inner.get_core_stop_error().await;
            assert_eq!(Fatal::Stopped, fatal);
            let result = raft.try_shutdown().await;
            let expected = Ok(());
            assert_eq!(expected, result);
        }
    });
}
