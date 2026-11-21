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
    use futures_util::FutureExt;
    use openraft_rt::testing::Suite;
    use tokio::runtime::Handle;
    use tokio::runtime::RuntimeFlavor;

    use super::*;

    #[test]
    fn test_tokio_rt() {
        Suite::<TokioRuntime>::test_all();
    }

    /// `TokioRuntime` uses the default `AsyncRuntime::spawn_blocking()`, which reports a panic.
    #[test]
    fn test_spawn_blocking_reports_a_panic() {
        let mut rt = TokioRuntime::new(2);
        rt.block_on(async {
            assert_eq!(7, TokioRuntime::spawn_blocking(|| 7).await.unwrap());

            let err = TokioRuntime::spawn_blocking(|| -> u32 { panic!("blocking probe") }).await.unwrap_err();
            assert_eq!(std::io::ErrorKind::Other, err.kind());
            assert_eq!("spawn_blocking worker panicked: blocking probe", err.to_string());

            let err = TokioRuntime::spawn_blocking(|| -> u32 { panic!("blocking probe {}", 2) }).await.unwrap_err();
            assert_eq!("spawn_blocking worker panicked: blocking probe 2", err.to_string());

            let err = TokioRuntime::spawn_blocking(|| -> u32 { std::panic::panic_any(5_u8) }).await.unwrap_err();
            assert_eq!(
                "spawn_blocking worker panicked: the panic payload is not a string",
                err.to_string()
            );
        });
    }

    /// Use up the cooperative budget of the current Tokio task. A Tokio receiver then reports a
    /// closed channel only on a later poll.
    fn use_up_task_budget() {
        while tokio::task::coop::has_budget_remaining() {
            let _ = tokio::task::coop::consume_budget().now_or_never();
        }
    }

    fn poll_once<F>(future: std::pin::Pin<&mut F>) -> std::task::Poll<F::Output>
    where F: Future {
        future.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
    }

    /// A cancellation that happened before a poll keeps that poll from polling the work, also when
    /// the task has used up its budget: a handle built on a Tokio channel reported the
    /// cancellation only on a later poll then.
    #[test]
    fn test_cancellable_without_task_budget() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;
        use std::task::Poll;

        fn counting<T>(polls: Arc<AtomicUsize>, output: Poll<T>) -> impl Future<Output = T>
        where T: Clone {
            std::future::poll_fn(move |_| {
                polls.fetch_add(1, Ordering::Relaxed);
                output.clone()
            })
        }

        let mut rt = TokioRuntime::new(1);
        rt.block_on(async {
            for drop_handle in [false, true] {
                let polls = Arc::new(AtomicUsize::new(0));
                let task = TokioRuntime::spawn(async move {
                    let (mut cancel, work) = TokioRuntime::cancellable(counting(polls.clone(), Poll::Ready(7)));
                    if drop_handle {
                        drop(cancel);
                    } else {
                        cancel.cancel();
                    }
                    use_up_task_budget();
                    let polled = poll_once(std::pin::pin!(work));
                    (polled, polls.load(Ordering::Relaxed))
                });
                assert_eq!(
                    (Poll::Ready(None), 0),
                    task.await.unwrap(),
                    "drop_handle: {drop_handle}"
                );
            }

            let polls = Arc::new(AtomicUsize::new(0));
            let task = TokioRuntime::spawn(async move {
                let (mut cancel, work) = TokioRuntime::cancellable(counting(polls.clone(), Poll::<u32>::Pending));
                let mut work = std::pin::pin!(work);
                assert_eq!(Poll::Pending, poll_once(work.as_mut()));
                cancel.cancel();
                use_up_task_budget();
                let polled = poll_once(work.as_mut());
                (polled, polls.load(Ordering::Relaxed))
            });
            assert_eq!((Poll::Ready(None), 1), task.await.unwrap(), "pending work");
        });
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
