use std::fmt;

use crate::RaftTypeConfig;
use crate::metrics::Metric;
use crate::metrics::metric_display::MetricDisplay;

/// A condition that the application wait for.
#[derive(Debug)]
pub(crate) enum Condition<C>
where C: RaftTypeConfig
{
    GE(Metric<C>),
    EQ(Metric<C>),
}

impl<C> Condition<C>
where C: RaftTypeConfig
{
    /// Build a new condition which the application will await to meet or exceed.
    pub(crate) fn ge(v: Metric<C>) -> Self {
        Self::GE(v)
    }

    /// Build a new condition which the application will await to meet.
    pub(crate) fn eq(v: Metric<C>) -> Self {
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

    pub(crate) fn value(&self) -> MetricDisplay<'_, C> {
        match self {
            Condition::GE(v) => v.value(),
            Condition::EQ(v) => v.value(),
        }
    }
}

impl<C> fmt::Display for Condition<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{}", self.name(), self.op(), self.value())
    }
}
