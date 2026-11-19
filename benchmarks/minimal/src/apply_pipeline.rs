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
#[openraft::macros::since(version = "0.10.1", change = "record actual apply stream consumption")]
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
    /// Input consumed by the call, including a prefix consumed before an error or cancellation.
    ApplyFinished {
        at: Instant,
        range: Option<Range<u64>>,
        entries: u64,
        payload_bytes: u64,
        contiguous: bool,
    },
}

/// A synchronous, benchmark-owned event sink; installed only for explicit diagnostics.
#[openraft::macros::since(version = "0.10.1")]
pub type Observer = dyn Fn(PipelineEvent) + Send + Sync;

/// An unfinished read or apply is reported when its future exits, including cancellation.
pub(crate) struct Span {
    observer: Option<Arc<Observer>>,
    read: bool,
    first: Option<u64>,
    last: Option<u64>,
    entries: u64,
    payload_bytes: u64,
    contiguous: bool,
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
        Self::new(observer, true)
    }

    pub(crate) fn apply(observer: Option<&Arc<Observer>>) -> Self {
        let observer = observer.cloned();
        if let Some(observer) = &observer {
            observer(PipelineEvent::ApplyStarted(Instant::now()));
        }
        Self::new(observer, false)
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

    fn new(observer: Option<Arc<Observer>>, read: bool) -> Self {
        Self {
            observer,
            read,
            first: None,
            last: None,
            entries: 0,
            payload_bytes: 0,
            contiguous: true,
        }
    }

    pub(crate) fn consume(&mut self, entry: &EntryOf<TypeConfig>) {
        if self.observer.is_none() {
            return;
        }
        let index = entry.log_id.index();
        if let Some(last) = self.last {
            self.contiguous &= last.checked_add(1) == Some(index);
        }
        if self.first.is_none() {
            self.first = Some(index);
        }
        self.last = Some(index);
        self.entries += 1;
        self.payload_bytes += payload_bytes(entry);
    }

    fn consumed_range(&self) -> Option<Range<u64>> {
        let first = self.first?;
        let last = self.last?;
        let end = last.checked_add(1)?;
        Some(first..end)
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
            let range = self.consumed_range();
            let representable = self.entries == 0 || range.is_some();
            PipelineEvent::ApplyFinished {
                at,
                range,
                entries: self.entries,
                payload_bytes: self.payload_bytes,
                contiguous: self.contiguous && representable,
            }
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

    use openraft::entry::RaftEntry;

    use super::*;

    type Events = Arc<Mutex<Vec<PipelineEvent>>>;
    type ReadEvent = (&'static str, Option<Range<u64>>, Option<(u64, u64)>);
    type ApplyConsumption = (Option<Range<u64>>, u64, u64, bool);

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

    fn consumed(events: &Events) -> Vec<ApplyConsumption> {
        let events = events.lock().unwrap();
        let consumed = events.iter().filter_map(|event| {
            let PipelineEvent::ApplyFinished {
                range,
                entries,
                payload_bytes,
                contiguous,
                ..
            } = event
            else {
                return None;
            };
            Some((range.clone(), *entries, *payload_bytes, *contiguous))
        });
        consumed.collect()
    }

    fn blank(index: u64) -> EntryOf<TypeConfig> {
        let log = openraft::testing::log_id::<TypeConfig>(1, 0, index);
        EntryOf::<TypeConfig>::new(log, EntryPayload::Blank)
    }

    #[test]
    fn reordering_is_visible_with_matching_endpoints_and_count() {
        let (observer, events) = observing();
        let mut span = Span::apply(Some(&observer));
        for index in [10, 12, 11, 13] {
            let entry = blank(index);
            span.consume(&entry);
        }
        drop(span);
        let actual = consumed(&events);
        let expected = vec![(Some(10..14), 4, 0, false)];
        assert_eq!(expected, actual);
    }

    #[test]
    fn dropping_an_apply_keeps_its_consumed_prefix() {
        let (observer, events) = observing();
        let mut span = Span::apply(Some(&observer));
        for index in [5, 6] {
            let entry = blank(index);
            span.consume(&entry);
        }
        drop(span);
        let actual = consumed(&events);
        let expected = vec![(Some(5..7), 2, 0, true)];
        assert_eq!(expected, actual);
    }

    #[test]
    fn an_empty_apply_has_no_invented_range() {
        let (observer, events) = observing();
        let span = Span::apply(Some(&observer));
        drop(span);
        let actual = consumed(&events);
        let expected = vec![(None, 0, 0, true)];
        assert_eq!(expected, actual);
    }
}
