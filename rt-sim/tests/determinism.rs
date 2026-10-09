//! Reproducibility and the executor behaviour the conformance suite does not pin down.

use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use openraft_rt::AsyncRuntime;
use openraft_rt::Instant;
use openraft_rt::Mpsc;
use openraft_rt::MpscReceiver;
use openraft_rt::MpscSender;
use openraft_rt::testing::Suite;
use openraft_rt_sim::SimInstant;
use openraft_rt_sim::SimRuntime;
use rand::RngExt;

fn suite_trace(seed: u64) -> Vec<String> {
    let mut rt = SimRuntime::with_seed(seed);
    rt.record_trace(true);
    Suite::<SimRuntime>::test_all_on(&mut rt);
    rt.take_trace()
}

#[test]
fn same_seed_same_trace() {
    let first = suite_trace(7);
    let second = suite_trace(7);

    for kind in ["spawn ", "poll ", "wake ", "timer+ ", "fire ", "advance ", "rng "] {
        assert!(
            first.iter().any(|event| event.starts_with(kind)),
            "trace has no {kind:?} event"
        );
    }
    if let Some(at) = first.iter().zip(&second).position(|(a, b)| a != b) {
        panic!("traces diverge at event {at}: {:?} vs {:?}", first[at], second[at]);
    }
    assert_eq!(first.len(), second.len());
}

#[test]
fn virtual_time_does_not_wait() {
    SimRuntime::new(1).block_on(async {
        let wall = std::time::Instant::now();
        let start = SimInstant::now();

        SimRuntime::sleep(Duration::from_secs(3600)).await;

        assert_eq!(SimInstant::now() - start, Duration::from_secs(3600));
        assert!(wall.elapsed() < Duration::from_secs(1));
    });
}

#[test]
fn equal_deadlines_fire_in_creation_order() {
    SimRuntime::new(1).block_on(async {
        let (tx, mut rx) = <SimRuntime as AsyncRuntime>::Mpsc::channel::<u32>(8);
        let deadline = SimInstant::now() + Duration::from_millis(5);
        let mut sleeps = Vec::new();
        for id in 0..4 {
            let sleep = SimRuntime::sleep_until(deadline);
            sleeps.push((id, sleep));
        }
        // Spawning in reverse polls the sleeps for the first time in reverse creation order.
        for (id, sleep) in sleeps.into_iter().rev() {
            let tx = tx.clone();
            let _detached = SimRuntime::spawn(async move {
                sleep.await;
                tx.send(id).await.unwrap();
            });
        }
        drop(tx);

        let mut order = Vec::new();
        while let Some(id) = rx.recv().await {
            order.push(id);
        }
        assert_eq!(order, vec![0, 1, 2, 3]);
    });
}

#[test]
fn task_panic_is_reported_through_join_handle() {
    SimRuntime::new(1).block_on(async {
        let handle = SimRuntime::spawn(async { panic!("boom") });
        let err = handle.await.unwrap_err();
        assert!(SimRuntime::is_panic(&err));
        assert_eq!(err.to_string(), "task panicked: boom");
    });
}

#[test]
fn spawn_blocking_is_unsupported() {
    SimRuntime::new(1).block_on(async {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_closure = ran.clone();

        let res = SimRuntime::spawn_blocking(move || ran_in_closure.store(true, Ordering::Relaxed)).await;

        let err = res.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert!(!ran.load(Ordering::Relaxed));
    });
}

#[test]
#[should_panic(expected = "rt-sim: deadlock")]
fn deadlock_panics_instead_of_hanging() {
    SimRuntime::new(1).block_on(std::future::pending::<()>());
}

#[test]
fn thread_rng_follows_the_seed() {
    let draws = |seed: u64| -> Vec<u64> {
        SimRuntime::with_seed(seed).block_on(async { (0..4).map(|_| SimRuntime::thread_rng().random()).collect() })
    };
    assert_eq!(draws(1), draws(1));
    assert_ne!(draws(1), draws(2));
}
