use std::fmt;

use crate::metrics::metric_display::MetricDisplay;
use crate::metrics::Metric;
use crate::NodeId;

/// A condition that the application wait for.
#[derive(Debug)]
pub(crate) enum Condition<NID>
where NID: NodeId
{
    GE(Metric<NID>),
    EQ(Metric<NID>),
}

impl<NID> Condition<NID>
where NID: NodeId
{
    /// Build a new condition which the application will await to meet or exceed.
    pub(crate) fn ge(v: Metric<NID>) -> Self {
        Self::GE(v)
    }

    /// Build a new condition which the application will await to meet.
    pub(crate) fn eq(v: Metric<NID>) -> Self {
        Self::EQ(v)
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Condition::GE(v) => v.name(),
            Condition::EQ(v) => v.name(),
        }
    }

    pub(crate) fn op(&self) -> &'static str {
        match self {
            Condition::GE(_) => ">=",
            Condition::EQ(_) => "==",
        }
    }

    pub(crate) fn value(&self) -> MetricDisplay<'_, NID> {
        match self {
            Condition::GE(v) => v.value(),
            Condition::EQ(v) => v.value(),
        }
    }
}

impl<NID> fmt::Display for Condition<NID>
where NID: NodeId
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{}", self.name(), self.op(), self.value())
    }
}
