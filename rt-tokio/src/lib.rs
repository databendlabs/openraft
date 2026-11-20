use std::future::Future;
use std::time::Duration;

use openraft_rt::AsyncRuntime;
use openraft_rt::OptionalSend;

mod instant;
mod mpsc;
mod mutex;
mod oneshot;
mod watch;

pub use instant::TokioInstant;
pub use mpsc::TokioMpsc;
pub use mpsc::TokioMpscReceiver;
pub use mpsc::TokioMpscSender;
pub use mpsc::TokioMpscWeakSender;
pub use mutex::TokioMutex;
pub use oneshot::TokioOneshot;
pub use oneshot::TokioOneshotSender;
pub use watch::TokioWatch;
pub use watch::TokioWatchReceiver;
pub use watch::TokioWatchSender;

/// `Tokio` is the default asynchronous executor.
///
/// On wasm targets, [`AsyncRuntime::new`] builds a current-thread runtime and ignores `threads`,
/// because tokio has no multi-thread runtime there.
///
/// When the `single-threaded` feature of `openraft-rt` is enabled, by this crate's
/// `single-threaded` feature or by any other crate, tasks need not be `Send`:
/// [`AsyncRuntime::spawn`] starts them with [`tokio::task::spawn_local`], and
/// [`AsyncRuntime::block_on`] runs its future on a [`tokio::task::LocalSet`].
///
/// # Running Openraft on the application's own runtime
///
/// When tasks need not be `Send`, [`AsyncRuntime::spawn`] calls `spawn_local`, which panics
/// outside a `LocalSet` or, from tokio 1.51, a `tokio::runtime::LocalRuntime`.
/// [`AsyncRuntime::block_on`] provides a `LocalSet`. An application that builds its own Tokio
/// runtime, such as with `Builder::new_current_thread()`, must run Openraft inside one of the two,
/// for example inside [`LocalSet::run_until`](tokio::task::LocalSet::run_until), or
/// [`LocalSet::block_on`](tokio::task::LocalSet::block_on) from synchronous code. The same code
/// also works when tasks are `Send`:
///
/// ```
/// use openraft_rt::AsyncRuntime;
/// use openraft_rt_tokio::TokioRuntime;
///
/// let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
/// let local = tokio::task::LocalSet::new();
///
/// rt.block_on(local.run_until(async {
///     // Run Openraft here.
///     let joined = TokioRuntime::spawn(async { 3 }).await;
///     assert_eq!(3, joined.unwrap());
/// }));
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct TokioRuntime {
    rt: tokio::runtime::Runtime,
    local: LocalTasks,
}

openraft_rt::__if_single_threaded! {
    {
        /// Runs the tasks that [`AsyncRuntime::spawn`] starts with `spawn_local`.
        type LocalTasks = tokio::task::LocalSet;
    } else {
        /// Takes the place of [`tokio::task::LocalSet`] when tasks are `Send`:
        /// [`AsyncRuntime::spawn`] starts them on the runtime, and [`AsyncRuntime::block_on`] needs
        /// no local task set.
        struct LocalTasks;

        impl LocalTasks {
            fn new() -> Self {
                LocalTasks
            }

            fn block_on<F>(&self, rt: &tokio::runtime::Runtime, future: F) -> F::Output
            where F: Future {
                rt.block_on(future)
            }
        }
    }
}

impl std::fmt::Debug for TokioRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokioRuntime").finish()
    }
}

impl AsyncRuntime for TokioRuntime {
    type JoinError = tokio::task::JoinError;
    type JoinHandle<T: OptionalSend + 'static> = tokio::task::JoinHandle<T>;
    type Sleep = tokio::time::Sleep;
    type Instant = TokioInstant;
    type TimeoutError = tokio::time::error::Elapsed;
    type Timeout<R, T: Future<Output = R> + OptionalSend> = tokio::time::Timeout<T>;
    type ThreadLocalRng = rand::rngs::ThreadRng;

    #[inline]
    fn spawn<T>(future: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + OptionalSend + 'static,
        T::Output: OptionalSend + 'static,
    {
        openraft_rt::__if_single_threaded!({ tokio::task::spawn_local(future) } else { tokio::task::spawn(future) })
    }

    #[inline]
    fn sleep(duration: Duration) -> Self::Sleep {
        tokio::time::sleep(duration)
    }

    #[inline]
    fn sleep_until(deadline: Self::Instant) -> Self::Sleep {
        tokio::time::sleep_until(deadline.0)
    }

    #[inline]
    fn timeout<R, F: Future<Output = R> + OptionalSend>(duration: Duration, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout(duration, future)
    }

    #[inline]
    fn timeout_at<R, F: Future<Output = R> + OptionalSend>(deadline: Self::Instant, future: F) -> Self::Timeout<R, F> {
        tokio::time::timeout_at(deadline.0, future)
    }

    #[inline]
    fn is_panic(join_error: &Self::JoinError) -> bool {
        join_error.is_panic()
    }

    #[inline]
    fn thread_rng() -> Self::ThreadLocalRng {
        rand::rng()
    }

    type Mpsc = TokioMpsc;
    type Watch = TokioWatch;
    type Oneshot = TokioOneshot;
    type Mutex<T: OptionalSend + 'static> = TokioMutex<T>;

    fn new(threads: usize) -> Self {
        #[cfg(not(target_family = "wasm"))]
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(threads)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");

        #[cfg(target_family = "wasm")]
        let rt = {
            let _ = threads;
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime")
        };

        TokioRuntime {
            rt,
            local: LocalTasks::new(),
        }
    }

    fn block_on<F, T>(&mut self, future: F) -> T
    where F: Future<Output = T> {
        self.local.block_on(&self.rt, future)
    }
}

#[cfg(test)]
mod tests {
    use openraft_rt::testing::Suite;
    use tokio::runtime::Handle;
    use tokio::runtime::RuntimeFlavor;

    use super::*;

    #[test]
    fn test_tokio_rt() {
        Suite::<TokioRuntime>::test_all();
    }

    #[test]
    fn test_new_runtime_flavor() {
        let mut rt = TokioRuntime::new(2);
        let (flavor, workers) = rt.block_on(async {
            let handle = Handle::current();
            (handle.runtime_flavor(), handle.metrics().num_workers())
        });

        #[cfg(not(target_family = "wasm"))]
        {
            assert_eq!(RuntimeFlavor::MultiThread, flavor);
            assert_eq!(2, workers);
        }
        #[cfg(target_family = "wasm")]
        {
            assert_eq!(RuntimeFlavor::CurrentThread, flavor);
            assert_eq!(1, workers);
        }
    }

    openraft_rt::__if_single_threaded! {
        {
            #[test]
            fn test_spawn_non_send_task() {
                let expected = std::rc::Rc::new(3);
                let value = expected.clone();

                let mut rt = TokioRuntime::new(2);
                let got = rt.block_on(async move { TokioRuntime::spawn(async move { value }).await });

                assert!(std::rc::Rc::ptr_eq(&expected, &got.unwrap()));
            }

            #[test]
            fn test_spawn_non_send_task_on_own_runtime() {
                let expected = std::rc::Rc::new(3);
                let value = expected.clone();

                // The application builds the runtime, and runs Openraft inside a `LocalSet`, as the
                // docs of `TokioRuntime` describe.
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let local = tokio::task::LocalSet::new();
                let got = rt.block_on(local.run_until(async move { TokioRuntime::spawn(async move { value }).await }));

                assert!(std::rc::Rc::ptr_eq(&expected, &got.unwrap()));
            }
        } else {}
    }
}
