//! Opt-in observations of the benchmark store's apply reads and state-machine calls.

use std::io;
use std::ops::Bound;
use std::ops::Range;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::time::Instant;

use openraft::alias::EntryOf;
use openraft::impls::EntryPayload;

use crate::store::TypeConfig;

/// Store boundaries, independent of the production worker's scheduling policy.
#[derive(Debug)]
#[openraft::macros::since(version = "0.10.1")]
pub enum PipelineEvent {
    ReadStarted {
        at: Instant,
        range: Option<Range<u64>>,
    },
    /// `entries` contains the actual entry count and referenced application-payload bytes.
    ReadFinished {
        at: Instant,
        entries: Option<(u64, u64)>,
        status: &'static str,
    },
    ApplyStarted(Instant),
    Applied {
        at: Instant,
        index: u64,
        entries: u64,
    },
    ApplyFinished(Instant),
}

/// A synchronous, benchmark-owned event sink; installed only for explicit diagnostics.
#[openraft::macros::since(version = "0.10.1")]
pub type Observer = dyn Fn(PipelineEvent) + Send + Sync;

/// An unfinished read or apply is reported when its future exits, including cancellation.
pub(crate) struct Span {
    observer: Option<Arc<Observer>>,
    read: bool,
}

impl Span {
    pub(crate) fn read<RB>(observer: Option<&Arc<Observer>>, range: &RB) -> Self
    where RB: RangeBounds<u64> {
        let observer = observer.cloned();
        if let Some(observer) = &observer {
            let at = Instant::now();
            let range = bounded_range(range);
            observer(PipelineEvent::ReadStarted { at, range });
        }
        Self { observer, read: true }
    }

    pub(crate) fn apply(observer: Option<&Arc<Observer>>) -> Self {
        let observer = observer.cloned();
        if let Some(observer) = &observer {
            observer(PipelineEvent::ApplyStarted(Instant::now()));
        }
        Self { observer, read: false }
    }

    pub(crate) fn finish_read(&mut self, result: &io::Result<Vec<EntryOf<TypeConfig>>>) {
        let Some(observer) = self.observer.take() else {
            return;
        };
        let at = Instant::now();
        let (entries, status) = match result {
            Ok(entries) => {
                let payload = entries.iter().map(payload_bytes).sum();
                (Some((entries.len() as u64, payload)), "completed")
            }
            Err(_) => (None, "failed"),
        };
        observer(PipelineEvent::ReadFinished { at, entries, status });
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(observer) = self.observer.take() else {
            return;
        };
        let at = Instant::now();
        let event = if self.read {
            PipelineEvent::ReadFinished {
                at,
                entries: None,
                status: "cancelled_or_panicked",
            }
        } else {
            PipelineEvent::ApplyFinished(at)
        };
        observer(event);
    }
}

fn bounded_range<RB>(range: &RB) -> Option<Range<u64>>
where RB: RangeBounds<u64> {
    let since = match range.start_bound() {
        Bound::Included(value) => *value,
        Bound::Excluded(value) => value.checked_add(1)?,
        Bound::Unbounded => 0,
    };
    let end = match range.end_bound() {
        Bound::Included(value) => value.checked_add(1)?,
        Bound::Excluded(value) => *value,
        Bound::Unbounded => return None,
    };
    Some(since..end)
}

fn payload_bytes(entry: &EntryOf<TypeConfig>) -> u64 {
    let EntryPayload::Normal(request) = &entry.payload else {
        return 0;
    };
    let Some(payload) = request.profile_payload() else {
        return 0;
    };
    payload.value.len() as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    #[cfg(feature = "realistic-profiles")]
    use openraft::entry::RaftEntry;

    use super::*;

    type Events = Arc<Mutex<Vec<PipelineEvent>>>;
    type ReadEvent = (&'static str, Option<Range<u64>>, Option<(u64, u64)>);

    fn observing() -> (Arc<Observer>, Events) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let observer: Arc<Observer> = Arc::new(move |event| captured.lock().unwrap().push(event));
        (observer, events)
    }

    fn read_events(events: &Events) -> Vec<ReadEvent> {
        let events = events.lock().unwrap();
        events
            .iter()
            .map(|event| match event {
                PipelineEvent::ReadStarted { range, .. } => ("started", range.clone(), None),
                PipelineEvent::ReadFinished { entries, status, .. } => (*status, None, *entries),
                _ => panic!("expected read events"),
            })
            .collect()
    }

    #[test]
    fn dropping_a_read_reports_cancellation_once() {
        let (observer, events) = observing();
        let span = Span::read(Some(&observer), &(5..8));
        drop(span);
        let expected = vec![("started", Some(5..8), None), ("cancelled_or_panicked", None, None)];
        let actual = read_events(&events);
        assert_eq!(expected, actual);
    }

    #[test]
    fn a_failed_read_is_distinct_from_cancellation() {
        let (observer, events) = observing();
        let mut span = Span::read(Some(&observer), &(5..8));
        let result = Err(io::Error::other("read failed"));
        span.finish_read(&result);
        drop(span);
        let expected = vec![("started", Some(5..8), None), ("failed", None, None)];
        let actual = read_events(&events);
        assert_eq!(expected, actual);
    }

    #[test]
    #[cfg(feature = "realistic-profiles")]
    fn successful_reads_count_actual_payload_lengths() -> anyhow::Result<()> {
        let profile = crate::profile::Profile {
            profile: true,
            payload_bytes: 4096,
            ..Default::default()
        };
        let request = profile.request(1)?;
        let log = openraft::testing::log_id::<TypeConfig>(1, 0, 5);
        let entry = EntryOf::<TypeConfig>::new(log, EntryPayload::Normal(request));
        let (observer, events) = observing();
        let mut span = Span::read(Some(&observer), &(5..6));
        span.finish_read(&Ok(vec![entry]));
        drop(span);
        let expected = vec![("started", Some(5..6), None), ("completed", None, Some((1, 4096)))];
        let actual = read_events(&events);
        assert_eq!(expected, actual);
        Ok(())
    }
}
