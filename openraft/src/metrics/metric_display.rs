use std::fmt;
use std::fmt::Formatter;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::metrics::Metric;

/// Display the value of a metric.
pub(crate) struct MetricDisplay<'a, C>
where C: RaftTypeConfig
{
    pub(crate) metric: &'a Metric<C>,
}

impl<C> fmt::Display for MetricDisplay<'_, C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.metric {
            Metric::Term(v) => write!(f, "{}", v),
            Metric::Vote(v) => write!(f, "{}", v),
            Metric::LastLogIndex(v) => write!(f, "{}", v.display()),
            Metric::Committed(v) => write!(f, "{}", v.display()),
            Metric::Applied(v) => write!(f, "{}", v.display()),
            Metric::AppliedIndex(v) => write!(f, "{}", v.display()),
            Metric::Snapshot(v) => write!(f, "{}", v.display()),
            Metric::Purged(v) => write!(f, "{}", v.display()),
        }
    }
}
