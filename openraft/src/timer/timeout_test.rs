use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::sleep;
use tokio::time::Instant;

use crate::timer::timeout::RaftTimer;
use crate::timer::Timeout;
use crate::TokioRuntime;

#[cfg(not(feature = "singlethreaded"))]
#[async_entry::test(worker_threads = 3)]
async fn test_timeout() -> anyhow::Result<()> {
    test_timeout_inner().await
}

#[cfg(feature = "singlethreaded")]
#[test]
fn test_timeout() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    tokio::task::LocalSet::new().block_on(&rt, test_timeout_inner())
}

async fn test_timeout_inner() -> anyhow::Result<()> {
    tracing::info!("--- set timeout, recv result");
    {
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        let _t = Timeout::<TokioRuntime>::new(
            || {
                let _ = tx.send(1u64);
            },
            Duration::from_millis(500),
        );

        let res = rx.await?;
        assert_eq!(1u64, res);

        let elapsed = now.elapsed();
        assert!(elapsed < Duration::from_millis(500 + 200));
        assert!(Duration::from_millis(500 - 200) < elapsed);
    }

    tracing::info!("--- update timeout");
    {
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        let t = Timeout::<TokioRuntime>::new(
            || {
                let _ = tx.send(1u64);
            },
            Duration::from_millis(500),
        );

        // Update timeout to 100 ms after 20 ms, the expected elapsed is 120 ms.
        sleep(Duration::from_millis(200)).await;
        t.update_timeout(Duration::from_millis(1000));

        let _res = rx.await?;

        let elapsed = now.elapsed();
        assert!(elapsed < Duration::from_millis(1200 + 200));
        assert!(Duration::from_millis(1200 - 200) < elapsed);
    }

    tracing::info!("--- update timeout to a lower value wont take effect");
    {
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        let t = Timeout::<TokioRuntime>::new(
            || {
                let _ = tx.send(1u64);
            },
            Duration::from_millis(500),
        );

        // Update timeout to 10 ms after 20 ms, the expected elapsed is still 50 ms.
        sleep(Duration::from_millis(200)).await;
        t.update_timeout(Duration::from_millis(100));

        let _res = rx.await?;

        let elapsed = now.elapsed();
        assert!(elapsed < Duration::from_millis(500 + 200));
        assert!(Duration::from_millis(500 - 200) < elapsed);
    }

    tracing::info!("--- drop the `Timeout` will cancel the callback");
    {
        let (tx, rx) = oneshot::channel();
        let now = Instant::now();
        let t = Timeout::<TokioRuntime>::new(
            || {
                let _ = tx.send(1u64);
            },
            Duration::from_millis(500),
        );

        // Drop the Timeout after 20 ms, the expected elapsed is 20 ms.
        sleep(Duration::from_millis(200)).await;
        drop(t);

        let res = rx.await;
        assert!(res.is_err());

        let elapsed = now.elapsed();
        assert!(elapsed < Duration::from_millis(200 + 100));
        assert!(Duration::from_millis(200 - 100) < elapsed);
    }

    Ok(())
}
