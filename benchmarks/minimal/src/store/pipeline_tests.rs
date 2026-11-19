use std::sync::Mutex;

use futures::FutureExt;
use openraft::Config;

use super::*;
use crate::apply_pipeline::PipelineEvent;
use crate::network::Router;

type Published = Arc<Mutex<Vec<(u64, u64, Instant)>>>;

fn record_published(events: &Published, event: PipelineEvent) {
    if let PipelineEvent::Applied { at, index, entries } = event {
        events.lock().unwrap().push((index, entries, at));
    }
}

fn observed_publication() -> anyhow::Result<(StateMachineStore, Published)> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let machine = StateMachineStore::new();
    let observing = events.clone();
    machine
        .pipeline_observer
        .set(Arc::new(move |event| record_published(&observing, event)))
        .map_err(|_| anyhow::anyhow!("pipeline observer already installed"))?;
    let observing = events.clone();
    machine
        .observe_applies(Arc::new(move |index, count, at| {
            observing.lock().unwrap().push((index, count, at));
        }))
        .map_err(anyhow::Error::msg)?;
    Ok((machine, events))
}

#[test]
fn pipeline_observation_preserves_the_existing_source_timestamp() -> anyhow::Result<()> {
    let (machine, events) = observed_publication()?;
    let log = openraft::testing::log_id::<TypeConfig>(2, 0, 5);
    machine.notify_applied(Some(log), 3);
    let actual = events.lock().unwrap().clone();
    let at = actual[0].2;
    let expected = vec![(5, 3, at), (5, 3, at)];
    assert_eq!(expected, actual);
    Ok(())
}

#[test]
fn an_absent_applied_id_creates_no_publication_observations() -> anyhow::Result<()> {
    let (machine, events) = observed_publication()?;
    machine.notify_applied(None, 0);
    let actual = events.lock().unwrap().clone();
    let expected: Vec<(u64, u64, Instant)> = Vec::new();
    assert_eq!(expected, actual);
    Ok(())
}

fn count_event(counts: &Mutex<[u64; 5]>, event: PipelineEvent) {
    let slot = match event {
        PipelineEvent::ReadStarted { .. } => 0,
        PipelineEvent::ReadFinished { .. } => 1,
        PipelineEvent::ApplyStarted(_) => 2,
        PipelineEvent::Applied { .. } => 3,
        PipelineEvent::ApplyFinished { .. } => 4,
    };
    counts.lock().unwrap()[slot] += 1;
}

#[tokio::test]
async fn installation_waits_for_an_apply_that_started_without_observers() -> anyhow::Result<()> {
    let mut router = Router::new();
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    router.new_cluster(Arc::new(config), std::collections::BTreeSet::from([0])).await?;
    let raft = router.get_raft(0);
    let machine = raft.with_state_machine(|sm| Box::pin(async move { sm.clone() })).await?;
    let locked = machine.sm.write().await;
    let (started, entered) = tokio::sync::oneshot::channel();
    *machine.apply_started.lock().unwrap() = Some(started);
    let writer = raft.clone();
    let writing = tokio::spawn(async move { writer.client_write(ClientRequest::default()).await });
    tokio::time::timeout(Duration::from_secs(5), entered).await??;

    let counts = Arc::new(Mutex::new([0; 5]));
    let observed = counts.clone();
    let observer = Arc::new(move |event| count_event(&observed, event));
    let mut installing = Box::pin(router.observe_apply_pipeline(0, observer));
    let polled = installing.as_mut().now_or_never();
    let pending = polled.is_none();
    assert!(pending);
    let installed = machine.pipeline_observer.get().is_some();
    assert!(!installed);
    drop(locked);
    writing.await??;
    installing.await?;
    let before = *counts.lock().unwrap();
    assert_eq!([0; 5], before);

    raft.client_write(ClientRequest::default()).await?;
    raft.with_state_machine(|_| Box::pin(async {})).await?;
    let after = *counts.lock().unwrap();
    assert_eq!([1; 5], after);
    let (_, writes) = machine.read().await;
    assert_eq!(2, writes);
    raft.shutdown().await?;
    Ok(())
}

type ApplyConsumption = (Option<std::ops::Range<u64>>, u64, u64, bool);
type Consumed = Arc<Mutex<Vec<ApplyConsumption>>>;

fn consuming_machine(profile: Option<Arc<Profile>>) -> anyhow::Result<(Arc<StateMachineStore>, Consumed)> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let mut machine = StateMachineStore::new();
    machine.profile = profile;
    let observer = Arc::new(move |event| {
        if let PipelineEvent::ApplyFinished {
            range,
            entries,
            payload_bytes,
            contiguous,
            ..
        } = event
        {
            captured.lock().unwrap().push((range, entries, payload_bytes, contiguous));
        }
    });
    machine.pipeline_observer.set(observer).map_err(|_| anyhow::anyhow!("observer already installed"))?;
    Ok((Arc::new(machine), events))
}

fn requested(index: u64) -> EntryResponder<TypeConfig> {
    let log = openraft::testing::log_id::<TypeConfig>(1, 0, index);
    let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(ClientRequest::default()));
    (entry, None)
}

#[tokio::test]
async fn stream_errors_keep_only_the_consumed_prefix() -> anyhow::Result<()> {
    let (mut machine, events) = consuming_machine(None)?;
    let entries = vec![
        Ok(requested(5)),
        Err(io::Error::other("stream failed")),
        Ok(requested(6)),
    ];
    let result = machine.apply(stream::iter(entries)).await;
    let error = result.expect_err("stream error is returned");
    let actual_error = error.to_string();
    assert_eq!("stream failed", actual_error);
    let actual = events.lock().unwrap().clone();
    let expected = vec![(Some(5..6), 1, 0, true)];
    assert_eq!(expected, actual);
    let state = machine.read().await;
    let log = openraft::testing::log_id::<TypeConfig>(1, 0, 5);
    assert_eq!((Some(log), 1), state);
    Ok(())
}

#[tokio::test]
async fn cancellation_keeps_consumed_stream_progress() -> anyhow::Result<()> {
    let (mut machine, events) = consuming_machine(None)?;
    let entries = stream::iter([Ok(requested(5))]).chain(stream::pending());
    let mut applying = Box::pin(machine.apply(entries));
    let first_poll = applying.as_mut().now_or_never();
    let pending = first_poll.is_none();
    assert!(pending);
    drop(applying);
    let actual = events.lock().unwrap().clone();
    let expected = vec![(Some(5..6), 1, 0, true)];
    assert_eq!(expected, actual);
    Ok(())
}

#[cfg(feature = "realistic-profiles")]
#[tokio::test]
async fn profile_errors_preserve_consumed_payload_without_publication() -> anyhow::Result<()> {
    let profile = Arc::new(Profile {
        profile: true,
        payload_bytes: 4096,
        ..Profile::default()
    });
    let (mut machine, events) = consuming_machine(Some(profile.clone()))?;
    let request = profile.request(7)?;
    let log = openraft::testing::log_id::<TypeConfig>(1, 0, 5);
    let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(request));
    let entries = vec![Ok((entry, None)), Err(io::Error::other("profile stream failed"))];
    let result = machine.apply(stream::iter(entries)).await;
    let error = result.expect_err("profile stream error is returned");
    let actual_error = error.to_string();
    assert_eq!("profile stream failed", actual_error);
    let actual = events.lock().unwrap().clone();
    let expected = vec![(Some(5..6), 1, 4096, true)];
    assert_eq!(expected, actual);
    let state = machine.read().await;
    assert_eq!((None, 0), state);
    Ok(())
}
