#[derive(Clone, Debug)]
#[derive(PartialEq)]
pub(crate) enum Expectation {
    /// Expect a replication to be at line rate, i.e, the lagging is less than
    /// `Config.replication_lag_threshold`.
    AtLineRate,
}
