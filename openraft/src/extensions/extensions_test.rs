use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use super::*;

#[test]
fn test_get_inserts_default() {
    let ext = Extensions::default();

    #[derive(Clone, Default, PartialEq, Debug)]
    struct MyData(u32);

    // First call creates default value
    let val = ext.get::<MyData>();
    assert_eq!(val, MyData(0));

    // Value is now stored
    assert!(ext.contains::<MyData>());

    // Second call returns same value
    let val2 = ext.get::<MyData>();
    assert_eq!(val2, MyData(0));
}

#[test]
fn test_get_stores_original_default() {
    #[derive(Clone)]
    struct Buffer(Vec<u8>);

    impl Default for Buffer {
        fn default() -> Self {
            Self(Vec::with_capacity(16))
        }
    }

    tracing::info!(capacity = 16, "check that the stored buffer keeps its reservation");
    {
        let ext = Extensions::default();
        assert_eq!(ext.get::<Buffer>().0.capacity(), 0);
        assert!(ext.remove::<Buffer>().unwrap().0.capacity() >= 16);
    }
}

#[test]
fn test_shared_state_with_arc() {
    let ext = Extensions::default();

    #[derive(Clone, Default)]
    struct Counter(Arc<AtomicU64>);

    // Get clone and mutate
    let counter1 = ext.get::<Counter>();
    counter1.0.fetch_add(1, Ordering::Relaxed);
    counter1.0.fetch_add(1, Ordering::Relaxed);

    // Get another clone - shares the same Arc
    let counter2 = ext.get::<Counter>();
    assert_eq!(counter2.0.load(Ordering::Relaxed), 2);

    // More increments
    counter2.0.fetch_add(1, Ordering::Relaxed);
    assert_eq!(counter1.0.load(Ordering::Relaxed), 3);
}

#[test]
fn test_remove() {
    let ext = Extensions::default();

    #[derive(Clone, Default, PartialEq, Debug)]
    struct MyData(u32);

    // Insert via get
    let _ = ext.get::<MyData>();
    assert!(ext.contains::<MyData>());

    // Remove
    assert_eq!(ext.remove::<MyData>(), Some(MyData(0)));
    assert!(!ext.contains::<MyData>());
    assert!(ext.remove::<MyData>().is_none());
}

#[test]
fn test_callbacks_access_other_types() {
    thread_local! {
        static EXT: Extensions = Extensions::default();
    }

    #[derive(Clone, Default, PartialEq, Debug)]
    struct Inner(u32);

    /// `default()` reads `Inner`, and `clone()` checks that `Inner` exists.
    #[derive(PartialEq, Debug)]
    struct Outer {
        inner: Inner,
        cloned_with_inner: bool,
    }

    impl Default for Outer {
        fn default() -> Self {
            Self {
                inner: EXT.with(|ext| ext.get::<Inner>()),
                cloned_with_inner: false,
            }
        }
    }

    impl Clone for Outer {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
                cloned_with_inner: EXT.with(|ext| ext.contains::<Inner>()),
            }
        }
    }

    tracing::info!("look up Outer on a worker thread, so that a deadlock fails the test instead of hanging it");
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let outer = EXT.with(|ext| ext.get::<Outer>());
            let _ = tx.send(());
            outer
        });

        let received = rx.recv_timeout(Duration::from_secs(10));
        assert_ne!(
            received,
            Err(RecvTimeoutError::Timeout),
            "a callback that accesses another type deadlocked"
        );

        let outer = worker.join().unwrap();
        assert_eq!(outer, Outer {
            inner: Inner(0),
            cloned_with_inner: true,
        });
    }
}

#[cfg(not(feature = "single-threaded"))]
#[test]
fn test_concurrent_first_lookups_store_one_default() {
    static DEFAULTS: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone)]
    struct Shared(Arc<AtomicU64>);

    impl Default for Shared {
        fn default() -> Self {
            DEFAULTS.fetch_add(1, Ordering::SeqCst);
            Self(Arc::new(AtomicU64::new(0)))
        }
    }

    let ext = Arc::new(Extensions::default());
    let threads = 8;

    tracing::info!(threads, "look up Shared from several threads at once");
    let values = {
        let barrier = Arc::new(std::sync::Barrier::new(threads));
        let workers = (0..threads)
            .map(|_| {
                let ext = ext.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    ext.get::<Shared>()
                })
            })
            .collect::<Vec<_>>();
        workers.into_iter().map(|w| w.join().unwrap()).collect::<Vec<_>>()
    };

    tracing::info!("check that every thread got a clone of the one stored default");
    {
        assert_eq!(DEFAULTS.load(Ordering::SeqCst), 1);
        assert!(values.iter().all(|v| Arc::ptr_eq(&v.0, &values[0].0)));

        let removed = ext.remove::<Shared>().unwrap();
        assert!(Arc::ptr_eq(&removed.0, &values[0].0));
    }
}

#[test]
fn test_panicking_default_leaves_other_types_usable() {
    #[derive(Clone)]
    struct Faulty;

    impl Default for Faulty {
        fn default() -> Self {
            panic!("Faulty::default() fails");
        }
    }

    #[derive(Clone, Default, PartialEq, Debug)]
    struct Healthy(u32);

    let ext = Extensions::default();

    tracing::info!("let the default of Faulty panic");
    {
        let res = std::panic::catch_unwind(AssertUnwindSafe(|| ext.get::<Faulty>()));
        assert!(res.is_err());
        assert!(!ext.contains::<Faulty>());
    }

    tracing::info!("check that another type still works");
    {
        assert_eq!(ext.get::<Healthy>(), Healthy(0));
        assert!(ext.contains::<Healthy>());
        assert_eq!(ext.remove::<Healthy>(), Some(Healthy(0)));
    }
}
