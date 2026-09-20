use std::future::poll_fn;
use std::time::Duration;

use openraft_rt::AsyncRuntime;

use super::*;
use crate::SimRuntime;

fn register_sleep(rt: &mut SimRuntime, sleep: &mut SimSleep) {
    rt.block_on(poll_fn(|cx| {
        let sleep = Pin::new(&mut *sleep);
        let polled = sleep.poll(cx);
        assert_eq!(polled, Poll::Pending);
        Poll::Ready(())
    }));
}

fn timer_events(rt: &SimRuntime) -> Vec<String> {
    let trace = rt.take_trace();
    let events = trace.into_iter().filter(|event| {
        let is_timer = event.starts_with("timer");
        let is_fire = event.starts_with("fire ");
        is_timer || is_fire
    });
    events.collect()
}

#[test]
fn drop_outside_runtime_cancels_timer() {
    let mut rt = SimRuntime::with_seed(0);
    rt.record_trace(true);
    let mut sleep = SimSleep::until(executor::EPOCH_NANOS + 1);
    register_sleep(&mut rt, &mut sleep);
    drop(sleep);
    rt.block_on(async {
        SimRuntime::sleep(Duration::from_nanos(2)).await;
    });
    let events = timer_events(&rt);
    let expected = [
        "timer+ (1,0) by t0",
        "timer- (1,0) by exec",
        "timer+ (2,1) by t0",
        "fire (2,1)",
    ];
    assert_eq!(events, expected);
}

#[test]
fn drop_in_another_runtime_cancels_only_owner_timer() {
    let mut owner = SimRuntime::with_seed(0);
    let mut other = SimRuntime::with_seed(0);
    owner.record_trace(true);
    other.record_trace(true);
    let mut sleep = SimSleep::until(executor::EPOCH_NANOS + 1);
    let mut other_sleep = SimSleep::until(executor::EPOCH_NANOS + 1);
    register_sleep(&mut owner, &mut sleep);
    register_sleep(&mut other, &mut other_sleep);
    other.block_on(async {
        drop(sleep);
        other_sleep.await;
    });
    let events = timer_events(&owner);
    let expected = ["timer+ (1,0) by t0", "timer- (1,0) by exec"];
    assert_eq!(events, expected);
    let events = timer_events(&other);
    let expected = ["timer+ (1,0) by t0", "fire (1,0)"];
    assert_eq!(events, expected);
}

#[test]
fn polling_another_runtime_moves_registration() {
    let mut first = SimRuntime::with_seed(0);
    let mut second = SimRuntime::with_seed(0);
    first.record_trace(true);
    second.record_trace(true);
    let mut sleep = SimSleep::until(executor::EPOCH_NANOS + 1);
    register_sleep(&mut first, &mut sleep);
    register_sleep(&mut first, &mut sleep);
    register_sleep(&mut second, &mut sleep);
    drop(sleep);
    let expected = ["timer+ (1,0) by t0", "timer- (1,0) by exec"];
    for rt in [&first, &second] {
        let events = timer_events(rt);
        assert_eq!(events, expected);
    }
}

#[test]
fn registration_does_not_keep_runtime_alive() {
    let mut rt = SimRuntime::with_seed(0);
    let owner = rt.block_on(async {
        let shared = executor::current();
        Arc::downgrade(&shared)
    });
    let mut sleep = SimSleep::until(executor::EPOCH_NANOS + 1);
    register_sleep(&mut rt, &mut sleep);
    drop(rt);
    let owner_alive = owner.upgrade().is_some();
    assert!(!owner_alive);
    drop(sleep);
}
