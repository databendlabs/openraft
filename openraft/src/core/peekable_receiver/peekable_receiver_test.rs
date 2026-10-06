use super::PeekableReceiver;
use crate::async_runtime::MpscSender;
use crate::async_runtime::Select2;
use crate::async_runtime::TryRecvError;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;

type C = UTConfig;

const CAPACITY: usize = 2;

#[test]
fn peek_preserves_order() -> anyhow::Result<()> {
    C::run(async {
        let (tx, inner) = C::mpsc(CAPACITY);
        let mut rx = PeekableReceiver::<C, u64>::new(inner);
        tx.send(10).await?;
        tx.send(20).await?;
        for _ in 0..2 {
            let peeked = rx.peek();
            assert_eq!(Ok(&10), peeked);
        }
        let first = rx.try_recv();
        assert_eq!(Ok(10), first);
        let second = rx.recv().await;
        assert_eq!(Some(20), second);
        Ok(())
    })
}

#[test]
fn peek_preserves_empty_and_closed_states() -> anyhow::Result<()> {
    C::run(async {
        let (tx, inner) = C::mpsc(CAPACITY);
        let mut rx = PeekableReceiver::<C, u64>::new(inner);
        let empty = rx.peek();
        assert_eq!(Err(TryRecvError::Empty), empty);
        tx.send(10).await?;
        let peeked = rx.peek();
        assert_eq!(Ok(&10), peeked);
        drop(tx);
        let buffered = rx.try_recv();
        assert_eq!(Ok(10), buffered);
        let closed = rx.peek();
        assert_eq!(Err(TryRecvError::Disconnected), closed);
        let received = rx.recv().await;
        assert_eq!(None, received);
        Ok(())
    })
}

#[test]
fn cancelled_receive_preserves_buffered_item() -> anyhow::Result<()> {
    C::run(async {
        let (tx, inner) = C::mpsc(CAPACITY);
        let mut rx = PeekableReceiver::<C, u64>::new(inner);
        tx.send(10).await?;
        let peeked = rx.peek();
        assert_eq!(Ok(&10), peeked);
        let shutdown = std::future::ready(());
        let receive = rx.recv();
        let selected = C::select2(shutdown, receive).await;
        assert_eq!(Select2::First(()), selected);
        let received = rx.recv().await;
        assert_eq!(Some(10), received);
        Ok(())
    })
}
