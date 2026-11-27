use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::RaftTypeConfig;
use crate::base::BoxFuture;
use crate::type_config::TypeConfigExt;

/// Sends the requests to one target one at a time, and never drops a request in flight.
///
/// Dropping a request in flight cancels it, and a transport over HTTP/2 resets its stream. A server
/// on the `h2` crate, as tonic's is, closes the connection with `GOAWAY` once more than 20 reset
/// streams, by default, wait for it to accept them. A server that stalls, such as a stopped
/// process, finds such a stream on resuming for each request that a timeout dropped meanwhile. So
/// a request stays in flight until it ends, and one sent meanwhile waits for it. A request that
/// waits replaces the one that waited before: that one has not started, so dropping it cancels
/// nothing.
///
/// Dropping a `SerialRpc` drops the request that waits. The request in flight runs on, on its own
/// task, until it ends.
pub(crate) struct SerialRpc<C>
where C: RaftTypeConfig
{
    queue: Arc<StdMutex<Queue>>,
    _p: PhantomData<fn() -> C>,
}

/// The requests of a [`SerialRpc`].
#[derive(Default)]
struct Queue {
    /// Whether a task is running a request.
    in_flight: bool,

    /// The request to run once the one in flight ends.
    waiting: Option<BoxFuture<'static, ()>>,
}

impl<C> SerialRpc<C>
where C: RaftTypeConfig
{
    pub(crate) fn new() -> Self {
        Self {
            queue: Default::default(),
            _p: PhantomData,
        }
    }

    /// Runs `request` on a new task if no request is in flight, or else once the one in flight
    /// ends, in place of a request that waits already.
    pub(crate) fn send(&self, request: BoxFuture<'static, ()>) {
        let mut queue = self.queue.lock().unwrap();
        if !queue.in_flight {
            queue.in_flight = true;
            drop(queue);
            self.spawn(request);
            return;
        }

        let replaced = queue.waiting.replace(request);
        drop(queue);
        // Dropped without the lock, as it drops a network client.
        drop(replaced);
    }

    /// Runs `request`, and then each request that waits, on a new task.
    fn spawn(&self, request: BoxFuture<'static, ()>) {
        let queue = self.queue.clone();
        let task = async move {
            let mut request = request;
            loop {
                request.await;

                request = {
                    let mut queue = queue.lock().unwrap();
                    match queue.waiting.take() {
                        Some(next) => next,
                        None => {
                            queue.in_flight = false;
                            return;
                        }
                    }
                };
            }
        };

        // False positive lint warning(`non-binding `let` on a future`): https://github.com/rust-lang/rust-clippy/issues/9932
        #[allow(clippy::let_underscore_future)]
        let _ = C::spawn(task);
    }
}

impl<C> Drop for SerialRpc<C>
where C: RaftTypeConfig
{
    fn drop(&mut self) {
        let waiting = self.queue.lock().unwrap().waiting.take();
        // Dropped without the lock, as it drops a network client.
        drop(waiting);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SerialRpc;
    use crate::async_runtime::MpscReceiver;
    use crate::async_runtime::MpscSender;
    use crate::async_runtime::OneshotSender;
    use crate::base::BoxFuture;
    use crate::engine::testing::UTConfig;
    use crate::type_config::TypeConfigExt;
    use crate::type_config::alias::MpscReceiverOf;
    use crate::type_config::alias::MpscSenderOf;
    use crate::type_config::alias::OneshotReceiverOf;
    use crate::type_config::alias::OneshotSenderOf;

    type C = UTConfig;

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    /// A request that reports when it starts and when it ends, and ends once its gate opens.
    ///
    /// It also returns a receiver that completes when the request is dropped, whether or not it
    /// ran.
    fn request(
        name: &'static str,
        log: &MpscSenderOf<C, String>,
    ) -> (BoxFuture<'static, ()>, OneshotSenderOf<C, ()>, OneshotReceiverOf<C, ()>) {
        let (open, gate) = C::oneshot();
        let (on_drop, dropped) = C::oneshot::<()>();
        let log = log.clone();
        let request = Box::pin(async move {
            let _on_drop = on_drop;
            log.send(format!("start {name}")).await.unwrap();
            gate.await.ok();
            log.send(format!("end {name}")).await.unwrap();
        });
        (request, open, dropped)
    }

    async fn next(log: &mut MpscReceiverOf<C, String>) -> String {
        C::timeout(TEST_TIMEOUT, log.recv()).await.unwrap().unwrap()
    }

    /// A request sent while another is in flight waits for it, and replaces the request that
    /// waited before, which never starts. After the last one ends, the next request starts at once.
    #[test]
    fn test_a_request_waits_for_the_one_in_flight() {
        C::run(async {
            let (log_tx, mut log) = C::mpsc(16);
            let rpc = SerialRpc::<C>::new();

            tracing::info!("--- a starts, then b waits for it, and c replaces b");
            let (open_a, open_c) = {
                let (a, open_a, _a_dropped) = request("a", &log_tx);
                let (b, _open_b, b_dropped) = request("b", &log_tx);
                let (c, open_c, _c_dropped) = request("c", &log_tx);

                rpc.send(a);
                assert_eq!("start a", next(&mut log).await);
                rpc.send(b);
                rpc.send(c);
                C::timeout(TEST_TIMEOUT, b_dropped).await.unwrap().ok();
                (open_a, open_c)
            };

            tracing::info!("--- c starts once a ends");
            {
                open_a.send(()).unwrap();
                assert_eq!("end a", next(&mut log).await);
                assert_eq!("start c", next(&mut log).await);
                open_c.send(()).unwrap();
                assert_eq!("end c", next(&mut log).await);
            }

            tracing::info!("--- with no request in flight, d starts at once");
            {
                let (d, open_d, _d_dropped) = request("d", &log_tx);
                rpc.send(d);
                assert_eq!("start d", next(&mut log).await);
                open_d.send(()).unwrap();
                assert_eq!("end d", next(&mut log).await);
            }
        });
    }

    /// Dropping a `SerialRpc` drops the request that waits, and the request in flight runs on
    /// until it ends.
    #[test]
    fn test_dropping_drops_the_waiting_request_only() {
        C::run(async {
            let (log_tx, mut log) = C::mpsc(16);
            let rpc = SerialRpc::<C>::new();

            let (a, open_a, a_dropped) = request("a", &log_tx);
            let (b, _open_b, b_dropped) = request("b", &log_tx);
            rpc.send(a);
            assert_eq!("start a", next(&mut log).await);
            rpc.send(b);

            tracing::info!("--- dropping the SerialRpc drops b");
            drop(rpc);
            C::timeout(TEST_TIMEOUT, b_dropped).await.unwrap().ok();

            tracing::info!("--- a runs on until its gate opens");
            open_a.send(()).unwrap();
            assert_eq!("end a", next(&mut log).await);
            C::timeout(TEST_TIMEOUT, a_dropped).await.unwrap().ok();
            drop(log_tx);
            assert_eq!(
                None,
                C::timeout(TEST_TIMEOUT, log.recv()).await.unwrap(),
                "b never starts"
            );
        });
    }
}
