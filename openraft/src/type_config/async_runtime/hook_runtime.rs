//! A test runtime: [`TokioRuntime`] with [`HookWatch`] as its watch channel.

use std::future::Future;
use std::time::Duration;

use openraft_rt_tokio::TokioRuntime;

use crate::OptionalSend;
use crate::async_runtime::AsyncRuntime;
use crate::type_config::async_runtime::hook_watch::HookWatch;

/// [`TokioRuntime`] with [`HookWatch`] as its watch channel, so that a test can place a send
/// between a change notification and the read that follows it.
#[derive(Debug)]
pub(crate) struct HookRuntime(TokioRuntime);

impl AsyncRuntime for HookRuntime {
    type JoinError = <TokioRuntime as AsyncRuntime>::JoinError;
    type JoinHandle<T: OptionalSend + 'static> = <TokioRuntime as AsyncRuntime>::JoinHandle<T>;
    type Sleep = <TokioRuntime as AsyncRuntime>::Sleep;
    type Instant = <TokioRuntime as AsyncRuntime>::Instant;
    type TimeoutError = <TokioRuntime as AsyncRuntime>::TimeoutError;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = <TokioRuntime as AsyncRuntime>::Timeout<R, T>;
    type ThreadLocalRng = <TokioRuntime as AsyncRuntime>::ThreadLocalRng;

    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        TokioRuntime::spawn(future)
    }

    fn sleep(duration: Duration) -> Self::Sleep {
        TokioRuntime::sleep(duration)
    }

    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        TokioRuntime::sleep_until(deadline)
    }

    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        TokioRuntime::timeout(duration, future)
    }

    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        TokioRuntime::timeout_at(deadline, future)
    }

    fn is_panic(join_error: &Self::JoinError) -> bool {
        TokioRuntime::is_panic(join_error)
    }

    fn thread_rng() -> Self::ThreadLocalRng {
        TokioRuntime::thread_rng()
    }

    type Mpsc = <TokioRuntime as AsyncRuntime>::Mpsc;
    type Watch = HookWatch;
    type Oneshot = <TokioRuntime as AsyncRuntime>::Oneshot;
    type Mutex<T: OptionalSend + 'static> = <TokioRuntime as AsyncRuntime>::Mutex<T>;

    fn new(threads: usize) -> Self {
        Self(TokioRuntime::new(threads))
    }

    fn block_on<F, T>(&mut self, future: F) -> T
    where F: Future<Output = T> {
        self.0.block_on(future)
    }
}
