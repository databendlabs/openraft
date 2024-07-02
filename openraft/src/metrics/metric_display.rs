use std::fmt;
use std::fmt::Formatter;

use crate::display_ext::DisplayOption;
use crate::metrics::Metric;
use crate::RaftTypeConfig;

/// Display the value of a metric.
pub(crate) struct MetricDisplay<'a, C>
where C: RaftTypeConfig
{
    pub(crate) metric: &'a Metric<C>,
}

impl<'a, C> fmt::Display for MetricDisplay<'a, C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.metric {
            Metric::Term(v) => write!(f, "{}", v),
            Metric::Vote(v) => write!(f, "{}", v),
            Metric::LastLogIndex(v) => write!(f, "{}", DisplayOption(v)),
            Metric::Applied(v) => write!(f, "{}", DisplayOption(v)),
            Metric::AppliedIndex(v) => write!(f, "{}", DisplayOption(v)),
            Metric::Snapshot(v) => write!(f, "{}", DisplayOption(v)),
            Metric::Purged(v) => write!(f, "{}", DisplayOption(v)),
        }
    }
}
