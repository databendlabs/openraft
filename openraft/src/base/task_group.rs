use std::future::Future;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;

use crate::OptionalSend;
use crate::async_runtime::AsyncRuntime;
use crate::async_runtime::CancelHandle;

/// Own spawned tasks until they finish, cancelling and joining peers after the first failure.
pub(crate) struct TaskGroup<Rt, E>
where
    Rt: AsyncRuntime,
    E: OptionalSend + 'static,
{
    cancellations: Vec<CancelHandle<Rt>>,
    tasks: FuturesUnordered<Rt::JoinHandle<Option<Result<(), E>>>>,
}

impl<Rt, E> TaskGroup<Rt, E>
where
    Rt: AsyncRuntime,
    E: OptionalSend + 'static,
{
    pub(crate) fn new() -> Self {
        Self {
            cancellations: Vec::new(),
            tasks: FuturesUnordered::new(),
        }
    }

    pub(crate) fn spawn<F>(&mut self, future: F)
    where F: Future<Output = Result<(), E>> + OptionalSend + 'static {
        let (cancel, future) = Rt::cancellable(future);
        let task = Rt::spawn(future);
        self.cancellations.push(cancel);
        self.tasks.push(task);
    }

    /// Preserve the first domain or join error, then drain every child before returning it.
    pub(crate) async fn join(mut self) -> Result<Result<(), E>, Rt::JoinError> {
        let mut failure = None;
        while let Some(joined) = self.tasks.next().await {
            let outcome = match joined {
                Ok(Some(result)) => Ok(result),
                Ok(None) => continue,
                Err(error) => Err(error),
            };
            let failed = match &outcome {
                Ok(result) => result.is_err(),
                Err(_) => true,
            };
            if failed && failure.is_none() {
                failure = Some(outcome);
                self.cancellations.clear();
            }
        }
        failure.unwrap_or(Ok(Ok(())))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::TaskGroup;
    use crate::RaftTypeConfig;
    use crate::async_runtime::OneshotSender;
    use crate::engine::testing::UTConfig;
    use crate::type_config::TypeConfigExt;

    type Runtime = <UTConfig as RaftTypeConfig>::AsyncRuntime;

    const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn failure_joins_cancelled_peers() {
        UTConfig::<()>::run(async {
            let mut tasks = TaskGroup::<Runtime, &'static str>::new();
            let (first, first_wait) = UTConfig::<()>::oneshot::<()>();
            let (second, second_wait) = UTConfig::<()>::oneshot::<()>();
            let (first_ready, ready_first) = UTConfig::<()>::oneshot();
            let (second_ready, ready_second) = UTConfig::<()>::oneshot();
            tasks.spawn(async move {
                first_ready.send(()).unwrap();
                let _released = first_wait.await;
                Ok(())
            });
            tasks.spawn(async move {
                second_ready.send(()).unwrap();
                let _released = second_wait.await;
                Ok(())
            });
            ready_first.await.unwrap();
            ready_second.await.unwrap();
            tasks.spawn(async { Err("injected stage failure") });

            let joining = tasks.join();
            let joined = UTConfig::<()>::timeout(WAIT_TIMEOUT, joining).await;
            let joined = joined.expect("all children stop before the timeout");
            let result = joined.expect("the tasks do not panic");
            assert_eq!(Err("injected stage failure"), result);
            let first_released = first.send(());
            assert_eq!(Err(()), first_released);
            let second_released = second.send(());
            assert_eq!(Err(()), second_released);
        });
    }

    #[test]
    fn success_waits_for_remaining_peers() {
        UTConfig::<()>::run(async {
            let mut tasks = TaskGroup::<Runtime, &'static str>::new();
            let (release, waiting) = UTConfig::<()>::oneshot();
            let (completed, completion) = UTConfig::<()>::oneshot();
            tasks.spawn(async { Ok(()) });
            let first_finished = async {
                loop {
                    let first = tasks.tasks.iter().next().expect("the first child exists");
                    if first.is_finished() {
                        break;
                    }
                    UTConfig::<()>::sleep(Duration::from_millis(1)).await;
                }
            };
            let finished = UTConfig::<()>::timeout(WAIT_TIMEOUT, first_finished).await;
            finished.expect("the successful child finishes before its peer is released");
            tasks.spawn(async move {
                let released = waiting.await;
                assert_eq!(Ok(()), released);
                completed.send("peer completed").unwrap();
                Ok(())
            });
            let mut joined = Box::pin(tasks.join());
            let polled = futures_util::poll!(joined.as_mut());
            let pending = polled.is_pending();
            assert!(pending, "successful EOF does not cancel a later stage");
            let sent = release.send(());
            assert_eq!(Ok(()), sent);
            let result = UTConfig::<()>::timeout(WAIT_TIMEOUT, joined).await;
            let result = result.expect("the remaining child finishes before the timeout");
            let result = result.expect("the tasks do not panic");
            assert_eq!(Ok(()), result);
            let completed = completion.await;
            assert_eq!(Ok("peer completed"), completed);
        });
    }

    #[test]
    fn panic_joins_cancelled_peer() {
        UTConfig::<()>::run(async {
            let mut tasks = TaskGroup::<Runtime, &'static str>::new();
            let (held, waiting) = UTConfig::<()>::oneshot::<()>();
            let (panic_now, trigger) = UTConfig::<()>::oneshot();
            tasks.spawn(async move {
                let _released = waiting.await;
                Ok(())
            });
            tasks.spawn(async move {
                let trigger = trigger.await;
                if trigger.is_ok() {
                    panic!("injected stage panic");
                }
                Ok(())
            });
            let triggered = panic_now.send(());
            assert_eq!(Ok(()), triggered);
            let joining = tasks.join();
            let joined = UTConfig::<()>::timeout(WAIT_TIMEOUT, joining).await;
            let joined = joined.expect("all children stop before the timeout");
            let panicked = joined.is_err();
            assert!(panicked, "a task panic remains a join error");
            let released = held.send(());
            assert_eq!(Err(()), released);
        });
    }
}
