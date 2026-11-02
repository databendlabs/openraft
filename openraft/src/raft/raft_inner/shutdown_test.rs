use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::FutureExt;

use super::RaftInner;
use crate::Config;
use crate::Extensions;
use crate::RaftMetrics;
use crate::async_runtime::OneshotSender;
use crate::config::RuntimeConfig;
use crate::core::TickHandle;
use crate::core::io_flush_tracking::IoProgressWatcher;
use crate::errors::Fatal;
use crate::errors::Infallible;
use crate::metrics::RaftServerMetrics;
use crate::raft::ShutdownReport;
use crate::raft::TaskJoinOutcome;
use crate::raft::core_state::CoreState;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinHandleOf;

crate::declare_raft_types!(UTConfig: AsyncRuntime = openraft_rt_tokio::TokioRuntime);

fn inner_with_tasks(
    core: JoinHandleOf<UTConfig, Result<Infallible, Fatal<UTConfig>>>,
    tick: JoinHandleOf<UTConfig, ()>,
) -> RaftInner<UTConfig> {
    let config = Arc::new(Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    });
    let (tx_shutdown, _rx_shutdown) = UTConfig::oneshot();
    let (tick_shutdown, _tick_shutdown_rx) = UTConfig::oneshot();
    RaftInner {
        id: 0,
        runtime_config: Arc::new(RuntimeConfig::new(&config)),
        config,
        tick_handle: Mutex::new(Some(TickHandle::new(tick_shutdown, tick))),
        tick_shutdown: Mutex::new(None),
        tx_api: UTConfig::mpsc(1).0,
        rx_metrics: UTConfig::watch_channel(RaftMetrics::new_initial(0)).1,
        rx_data_metrics: UTConfig::watch_channel(Default::default()).1,
        rx_server_metrics: UTConfig::watch_channel(RaftServerMetrics::new_initial(0)).1,
        progress_watcher: IoProgressWatcher::new().1,
        tx_shutdown: Mutex::new(Some(tx_shutdown)),
        core_state: Mutex::new(CoreState::Running(core)),
        extensions: Extensions::default(),
    }
}

#[test]
fn shutdown_report_clean_and_repeated() {
    UTConfig::run(async {
        let inner = inner_with_tasks(
            UTConfig::spawn(async { Err(Fatal::Stopped) }),
            UTConfig::spawn(async {}),
        );

        tracing::info!("join both tasks and record their outcomes");
        let report = {
            let report = inner.shutdown_with_report().await;
            assert_eq!(TaskJoinOutcome::Joined, report.core);
            assert_eq!(TaskJoinOutcome::Joined, report.tick);
            assert!(!report.core_already_joined);
            assert!(!report.tick_already_joined);
            report
        };

        tracing::info!("a repeated shutdown must identify the earlier joins");
        {
            assert_eq!(
                ShutdownReport {
                    core_already_joined: true,
                    tick_already_joined: true,
                    ..report
                },
                inner.shutdown_with_report().await
            );
        }
    });
}

#[test]
fn shutdown_report_core_panic_after_internal_join() {
    UTConfig::run(async {
        tracing::info!("retain the core panic even when a request joins it before shutdown");
        {
            let inner = inner_with_tasks(
                UTConfig::spawn(async { panic!("injected core task panic") }),
                UTConfig::spawn(async {}),
            );
            assert_eq!(Fatal::Panicked, inner.get_core_stop_error().await);
            let report = inner.shutdown_with_report().await;
            assert_eq!(TaskJoinOutcome::Panicked, report.core);
            assert!(report.core_already_joined);
            assert_eq!(TaskJoinOutcome::Joined, report.tick);
            assert_eq!(TaskJoinOutcome::Panicked, inner.shutdown_with_report().await.core);
        }
    });
}

#[test]
fn shutdown_report_tick_panic() {
    UTConfig::run(async {
        tracing::info!("surface a tick task panic and retain it across shutdown calls");
        {
            let inner = inner_with_tasks(
                UTConfig::spawn(async { Err(Fatal::Stopped) }),
                UTConfig::spawn(async { panic!("injected tick task panic") }),
            );
            let report = inner.shutdown_with_report().await;
            assert_eq!(TaskJoinOutcome::Joined, report.core);
            assert_eq!(TaskJoinOutcome::Panicked, report.tick);
            assert_eq!(TaskJoinOutcome::Panicked, inner.shutdown_with_report().await.tick);
        }
    });
}

#[test]
fn shutdown_report_aborted_tasks() {
    UTConfig::run(async {
        tracing::info!("abort both tasks to distinguish cancellation from a successful join");
        {
            let core = UTConfig::spawn(std::future::pending());
            let tick = UTConfig::spawn(std::future::pending());
            core.abort();
            tick.abort();
            let inner = inner_with_tasks(core, tick);
            let report = inner.shutdown_with_report().await;
            assert_eq!(TaskJoinOutcome::Failed, report.core);
            assert_eq!(TaskJoinOutcome::Failed, report.tick);
            assert_eq!(TaskJoinOutcome::Failed, inner.shutdown_with_report().await.core);
        }
    });
}

#[test]
fn shutdown_report_resumes_cancelled_join() {
    UTConfig::run(async {
        let (core_tx, core_rx) = UTConfig::oneshot();
        let (tick_tx, tick_rx) = UTConfig::oneshot();
        let inner = inner_with_tasks(
            UTConfig::spawn(async move {
                core_rx.await.unwrap();
                Err(Fatal::Stopped)
            }),
            UTConfig::spawn(async move { tick_rx.await.unwrap() }),
        );

        tracing::info!("drop a shutdown waiting on the core; its join must remain available");
        {
            assert!(inner.shutdown_with_report().now_or_never().is_none());
            core_tx.send(()).unwrap();
            UTConfig::timeout(Duration::from_secs(1), inner.join_core_task()).await.unwrap();
        }

        tracing::info!("drop a shutdown waiting on the tick; subsequent callers share that join");
        {
            assert!(inner.shutdown_with_report().now_or_never().is_none());
            tick_tx.send(()).unwrap();
            let (first, second) = UTConfig::timeout(Duration::from_secs(1), async {
                futures_util::future::join(inner.shutdown_with_report(), inner.shutdown_with_report()).await
            })
            .await
            .unwrap();
            assert_eq!(TaskJoinOutcome::Joined, first.core);
            assert_eq!(TaskJoinOutcome::Joined, first.tick);
            assert_eq!(first.core, second.core);
            assert_eq!(first.tick, second.tick);
        }
    });
}
