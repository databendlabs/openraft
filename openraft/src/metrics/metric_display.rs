use std::fmt;
use std::fmt::Formatter;

use crate::display_ext::DisplayOption;
use crate::metrics::Metric;
use crate::NodeId;

/// Display the value of a metric.
pub(crate) struct MetricDisplay<'a, NID>
where NID: NodeId
{
    pub(crate) metric: &'a Metric<NID>,
}

impl<'a, NID> fmt::Display for MetricDisplay<'a, NID>
where NID: NodeId
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
