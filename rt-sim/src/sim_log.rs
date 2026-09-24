//! A reproducible tracing log, written when `OPENRAFT_SIM_LOG` names a file.
//!
//! A regular log is stamped with the wall clock and the thread id, so no two runs match. While
//! [`block_on`](crate::SimRuntime) runs, this module routes the events of its thread to a file
//! instead, through a thread-local default subscriber: each event is stamped with virtual time,
//! thread and span ids are left out, and the wall-clock rendering of instants is masked
//! (`DisplayInstant` converts through `SystemTime::now()`). Two runs with the same seed write the
//! same file.
//!
//! The level comes from `RUST_LOG`, and defaults to `DEBUG`. Every runtime in the process appends
//! to the same file, which is truncated when the first one starts.

use std::fmt::Write as _;
use std::fs::File;
use std::sync::Mutex;
use std::sync::OnceLock;

use tracing::Dispatch;
use tracing::Event;
use tracing::Subscriber;
use tracing::dispatcher::DefaultGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Registry;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::FormatEvent;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::fmt::FormattedFields;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;

use crate::SimInstant;

/// Names the file the reproducible log is written to. Unset, nothing is installed.
pub const SIM_LOG_ENV: &str = "OPENRAFT_SIM_LOG";

/// Makes the reproducible log the default subscriber of this thread until the guard drops.
/// Returns `None` when [`SIM_LOG_ENV`] is unset.
pub(crate) fn enter() -> Option<DefaultGuard> {
    static DISPATCH: OnceLock<Option<Dispatch>> = OnceLock::new();

    let dispatch = DISPATCH.get_or_init(|| {
        let path = std::env::var_os(SIM_LOG_ENV)?;
        let file = File::create(&path).unwrap_or_else(|e| panic!("{SIM_LOG_ENV}={path:?}: {e}"));
        let directives = std::env::var(EnvFilter::DEFAULT_ENV).unwrap_or_else(|_| "DEBUG".to_string());
        let layer = fmt::Layer::new().with_writer(Mutex::new(file)).with_ansi(false).event_format(SimEventFormatter);
        Some(Dispatch::new(
            Registry::default().with(EnvFilter::new(directives)).with(layer),
        ))
    });
    dispatch.as_ref().map(tracing::dispatcher::set_default)
}

struct SimEventFormatter;

impl<S, N> FormatEvent<S, N> for SimEventFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(&self, ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> std::fmt::Result {
        let mut line = String::new();

        match SimInstant::try_now() {
            Some(now) => {
                let elapsed = now.elapsed_since_start();
                write!(line, "{}.{:09}", elapsed.as_secs(), elapsed.subsec_nanos())?;
            }
            None => line.push('-'),
        }
        write!(line, " {:>5} ", event.metadata().level().as_str())?;

        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                write!(line, "{}@{}", span.metadata().target(), span.metadata().name())?;
                let ext = span.extensions();
                if let Some(fields) = &ext.get::<FormattedFields<N>>()
                    && !fields.is_empty()
                {
                    write!(line, "{{{}}}", fields)?;
                }
                line.push_str(": ");
            }
        }

        ctx.format_fields(Writer::new(&mut line), event)?;
        writeln!(writer, "{}", mask_wall_clock(&line))
    }
}

/// Replaces the two formats `DisplayInstant` renders a wall-clock time in: `%H:%M:%S%.6f` and
/// `%Y-%m-%dT%H:%M:%S%.6fZ%z`.
fn mask_wall_clock(line: &str) -> String {
    // '9' stands for a digit and 'S' for a sign; every other byte must match literally.
    const FULL: &[u8] = b"9999-99-99T99:99:99.999999ZS9999";
    const SIMPLE: &[u8] = b"99:99:99.999999";

    fn matches_at(bytes: &[u8], at: usize, pattern: &[u8]) -> bool {
        bytes.len() >= at + pattern.len()
            && pattern.iter().zip(&bytes[at..]).all(|(p, b)| match p {
                b'9' => b.is_ascii_digit(),
                b'S' => *b == b'+' || *b == b'-',
                _ => p == b,
            })
    }

    let bytes = line.as_bytes();
    let mut masked = String::with_capacity(line.len());
    let mut copied_to = 0;
    let mut at = 0;
    while at < bytes.len() {
        let len = if matches_at(bytes, at, FULL) {
            FULL.len()
        } else if matches_at(bytes, at, SIMPLE) {
            SIMPLE.len()
        } else {
            at += 1;
            continue;
        };
        masked.push_str(&line[copied_to..at]);
        masked.push_str("<wall-clock>");
        at += len;
        copied_to = at;
    }
    masked.push_str(&line[copied_to..]);
    masked
}

#[cfg(test)]
mod tests {
    use super::mask_wall_clock;

    #[test]
    fn masks_both_instant_formats() {
        assert_eq!(
            mask_wall_clock("sent at 08:12:33.123456, lease 2026-09-14T08:12:33.123456Z+0300 ok"),
            "sent at <wall-clock>, lease <wall-clock> ok"
        );
        assert_eq!(
            mask_wall_clock("0.000006000 DEBUG no instants: 12:3"),
            "0.000006000 DEBUG no instants: 12:3"
        );
    }
}
