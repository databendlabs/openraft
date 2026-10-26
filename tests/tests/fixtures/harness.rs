#[cfg(feature = "bt")]
use std::backtrace::Backtrace;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::env;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::panic::PanicHookInfo;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Once;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use lazy_static::lazy_static;
use openraft::OptionalSend;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;
use tracing_appender::non_blocking::WorkerGuard;

use crate::fixtures::logging::init_file_logging;

const TEST_TIMEOUT: Duration = Duration::from_secs(120);

// The harness polls the test's main future on this thread on both runtimes.
thread_local! {
    static NETWORK_DELAYS: RefCell<Vec<Arc<AtomicU64>>> = const { RefCell::new(Vec::new()) };
}

/// Retain only the delay setting so failure reports include later overrides.
pub(super) fn record_network_delay(delay: &Arc<AtomicU64>) {
    NETWORK_DELAYS.with(|delays| delays.borrow_mut().push(delay.clone()));
}

fn report_failure<F>() {
    let test = std::any::type_name::<F>();
    let test = test.trim_end_matches("::{{closure}}");
    let logs = env::current_dir().expect("test working directory");
    let logs = logs.join("_log");
    let seed = env::var_os("OPENRAFT_RT_SIM_SEED");
    let delays: BTreeSet<_> = NETWORK_DELAYS.with(|delays| {
        let delays = delays.borrow();
        let values = delays.iter().map(|delay| delay.load(Ordering::Relaxed));
        values.collect()
    });
    eprintln!(
        "test: {test}\nlogs: {}/ut.*\nsimulation seed: {seed:?} (default 0)\nnetwork delays (ms): {delays:?}",
        logs.display()
    );
}

/// Create a harness that sets up tracing and an async runtime for testing.
pub fn ut_harness<F, Fut>(f: F) -> anyhow::Result<()>
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = anyhow::Result<()>> + OptionalSend + 'static,
{
    #[allow(clippy::let_unit_value)]
    let _g = init_default_ut_tracing();

    NETWORK_DELAYS.with(|delays| delays.borrow_mut().clear());
    let run = || {
        let duration = match env::var("OPENRAFT_TEST_TIMEOUT_SECS") {
            Ok(value) => {
                let seconds = value.parse().context("invalid OPENRAFT_TEST_TIMEOUT_SECS")?;
                Duration::from_secs(seconds)
            }
            Err(env::VarError::NotPresent) => TEST_TIMEOUT,
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(!duration.is_zero(), "OPENRAFT_TEST_TIMEOUT_SECS must be positive");
        TypeConfig::run(async move {
            let result = TypeConfig::timeout(duration, f()).await;
            result.context("test exceeded its deadline")?
        })
    };
    let result = std::panic::catch_unwind(AssertUnwindSafe(run));
    match result {
        Ok(result) => {
            if let Err(error) = &result {
                report_failure::<F>();
                tracing::error!(?error, "test failed");
            }
            result
        }
        Err(panic) => {
            report_failure::<F>();
            std::panic::resume_unwind(panic)
        }
    }
}

pub fn init_default_ut_tracing() {
    static START: Once = Once::new();

    START.call_once(|| {
        let mut g = GLOBAL_UT_LOG_GUARD.as_ref().lock().unwrap();
        *g = Some(init_global_tracing("ut", "_log", "DEBUG"));
    });
}

lazy_static! {
    static ref GLOBAL_UT_LOG_GUARD: Arc<Mutex<Option<WorkerGuard>>> = Arc::new(Mutex::new(None));
}

pub fn init_global_tracing(app_name: &str, dir: &str, level: &str) -> WorkerGuard {
    set_panic_hook();

    let (g, sub) = init_file_logging(app_name, dir, level);
    tracing::subscriber::set_global_default(sub).expect("error setting global tracing subscriber");

    tracing::info!("initialized global tracing: in {}/{} at {}", dir, app_name, level);
    g
}

pub fn set_panic_hook() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        log_panic(panic);
        prev_hook(panic);
    }));
}

pub fn log_panic(panic: &PanicHookInfo) {
    let backtrace = {
        #[cfg(feature = "bt")]
        {
            format!("{:?}", Backtrace::force_capture())
        }

        #[cfg(not(feature = "bt"))]
        {
            "backtrace is disabled without --features 'bt'".to_string()
        }
    };

    if let Some(location) = panic.location() {
        tracing::error!(
            message = %panic.to_string().replace('\n', " "),
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
    } else {
        tracing::error!(message = %panic.to_string().replace('\n', " "), backtrace = %backtrace);
    }
}
