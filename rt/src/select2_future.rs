use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use pin_project_lite::pin_project;

use crate::Select2;

pin_project! {
    /// The future that [`AsyncRuntime::select2()`] returns by default.
    ///
    /// It holds each of the two futures once, in a pinned field. An `async` block awaiting
    /// them would also keep the futures it captured by value, which doubles their size:
    /// <https://github.com/rust-lang/rust/issues/62958>.
    ///
    /// [`AsyncRuntime::select2()`]: crate::AsyncRuntime::select2
    #[project = Select2FutureProj]
    #[project_replace = Select2FutureProjReplace]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub(crate) enum Select2Future<A, B> {
        Pending {
            #[pin]
            a: A,
            #[pin]
            b: B,
        },
        Done,
    }
}

impl<A, B> Select2Future<A, B> {
    pub(crate) fn new(a: A, b: B) -> Self {
        Self::Pending { a, b }
    }
}

impl<A, B> Future for Select2Future<A, B>
where
    A: Future,
    B: Future,
{
    type Output = Select2<A::Output, B::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Select2FutureProj::Pending { a, b } = self.as_mut().project() else {
            panic!("Select2Future polled after it completed");
        };

        let selected = if let Poll::Ready(value) = a.poll(cx) {
            Select2::First(value)
        } else if let Poll::Ready(value) = b.poll(cx) {
            Select2::Second(value)
        } else {
            return Poll::Pending;
        };

        // Drop both futures now, as an `async` block does when it returns.
        self.project_replace(Self::Done);
        Poll::Ready(selected)
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::future::pending;
    use std::future::ready;
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::Context;
    use std::task::Poll;

    use futures_util::task::noop_waker_ref;

    use super::Select2Future;
    use crate::Select2;

    /// Hold 1 KiB while waiting forever, so that the future takes a little over 1 KiB.
    async fn kib_future() {
        let buf = [7u8; 1024];
        pending::<()>().await;
        std::hint::black_box(&buf);
    }

    #[test]
    fn test_holds_each_future_once() {
        let one = size_of_val(&kib_future());
        let selected = Select2Future::new(kib_future(), kib_future());

        assert!(
            size_of_val(&selected) < 2 * one + 64,
            "{} bytes for two {one}-byte futures",
            size_of_val(&selected)
        );
    }

    #[test]
    fn test_polls_the_first_future_first() {
        let mut cx = Context::from_waker(noop_waker_ref());

        let mut both_ready = pin!(Select2Future::new(ready(1), ready("two")));
        assert_eq!(Poll::Ready(Select2::First(1)), both_ready.as_mut().poll(&mut cx));

        let mut second_ready = pin!(Select2Future::new(pending::<u8>(), ready("two")));
        assert_eq!(Poll::Ready(Select2::Second("two")), second_ready.as_mut().poll(&mut cx));

        let mut none_ready = pin!(Select2Future::new(pending::<u8>(), pending::<u8>()));
        assert_eq!(Poll::Pending, none_ready.as_mut().poll(&mut cx));
    }

    #[test]
    fn test_drops_both_futures_when_it_completes() {
        let mut cx = Context::from_waker(noop_waker_ref());
        let marker = Arc::new(());

        let losing = {
            let marker = marker.clone();
            async move {
                pending::<()>().await;
                drop(marker);
            }
        };
        let mut selected = pin!(Select2Future::new(ready(1), losing));

        assert_eq!(Poll::Ready(Select2::First(1)), selected.as_mut().poll(&mut cx));
        assert_eq!(
            1,
            Arc::strong_count(&marker),
            "the losing future is dropped before the select is"
        );
    }
}
