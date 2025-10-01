use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;

use validit::Validate;

use crate::LogIdOptionExt;
use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::type_config::alias::LogIdOf;

// TODO: I need just a range, but not a log id range.

/// A log id range of continuous series of log entries.
///
/// The range of log to send is left open right close: `(prev, last]`.
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct LogIdRange<C>
where C: RaftTypeConfig
{
    /// The prev log id before the first to send, exclusive.
    pub(crate) prev: Option<LogIdOf<C>>,

    /// The last log id to send, inclusive.
    pub(crate) last: Option<LogIdOf<C>>,
}

impl<C> Copy for LogIdRange<C>
where
    C: RaftTypeConfig,
    LogIdOf<C>: Copy,
{
}

impl<C> Display for LogIdRange<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}]", self.prev.display(), self.last.display())
    }
}

impl<C> Validate for LogIdRange<C>
where C: RaftTypeConfig
{
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        validit::less_equal!(&self.prev, &self.last);
        Ok(())
    }
}

impl<C> LogIdRange<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(prev: Option<LogIdOf<C>>, last: Option<LogIdOf<C>>) -> Self {
        Self { prev, last }
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> u64 {
        self.last.next_index() - self.prev.next_index()
    }
}

#[cfg(test)]
mod tests {
    use validit::Valid;

    use crate::engine::testing::UTConfig;
    use crate::log_id_range::LogIdRange;
    use crate::type_config::alias::LogIdOf;

    fn log_id(index: u64) -> LogIdOf<UTConfig> {
        crate::engine::testing::log_id(1, 1, index)
    }

    #[test]
    fn test_log_id_range_validate() -> anyhow::Result<()> {
        let res = std::panic::catch_unwind(|| {
            let r = Valid::new(LogIdRange::<UTConfig>::new(Some(log_id(5)), None));
            let _x = &r.last;
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "prev(5) > last(None)");

        let res = std::panic::catch_unwind(|| {
            let r = Valid::new(LogIdRange::<UTConfig>::new(Some(log_id(5)), Some(log_id(4))));
            let _x = &r.last;
        });
        tracing::info!("res: {:?}", res);
        assert!(res.is_err(), "prev(5) > last(4)");

        Ok(())
    }
}
