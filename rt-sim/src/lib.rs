//! A deterministic, simulated [`AsyncRuntime`](openraft_rt::AsyncRuntime) for testing Openraft.

mod executor;
mod instant;
mod join;
mod runtime;
#[cfg(feature = "sim-log")]
mod sim_log;
mod sleep;
mod timeout;
mod watch;

pub use instant::SimInstant;
pub use join::SimJoinError;
pub use join::SimJoinHandle;
pub use runtime::SEED_ENV;
pub use runtime::SimRuntime;
pub use runtime::TRACE_ENV;
#[cfg(feature = "sim-log")]
pub use sim_log::SIM_LOG_ENV;
pub use sleep::SimSleep;
pub use timeout::Elapsed;
pub use timeout::SimTimeout;
pub use watch::SimWatch;
pub use watch::SimWatchReceiver;
pub use watch::SimWatchRef;
pub use watch::SimWatchSender;
