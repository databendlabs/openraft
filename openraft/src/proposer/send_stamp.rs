use std::fmt;

use crate::Instant;
use crate::display_ext::DisplayInstantExt;

/// Marks an RPC sent by the Leader with its send time and heartbeat round.
///
/// Stamps compare by `time` first and by `heartbeat_round` second, as the field order below
/// defines. The round orders RPCs that share a `time`: two `now()` calls may return the same value,
/// for example on a runtime whose clock stands still while tasks run. A heartbeat broadcast after a
/// read arrives has a higher round than the read recorded, so its stamp exceeds the read's
/// threshold even if the clock has not moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SendStamp<I> {
    /// When the RPC was sent.
    pub(crate) time: I,

    /// The heartbeat round of a heartbeat RPC, or `0` for any other RPC.
    pub(crate) heartbeat_round: u64,
}

impl<I> SendStamp<I> {
    pub(crate) fn new(time: I, heartbeat_round: u64) -> Self {
        Self { time, heartbeat_round }
    }
}

impl<I> fmt::Display for SendStamp<I>
where I: Instant
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(heartbeat_round={})", self.time.display(), self.heartbeat_round)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SendStamp;
    use crate::engine::testing::UTConfig;
    use crate::type_config::TypeConfigExt;

    #[test]
    fn test_order_by_time_then_heartbeat_round() {
        let t1 = UTConfig::<()>::now();
        let t2 = t1 + Duration::from_millis(1);

        let replication_at_t1 = SendStamp::new(t1, 0);
        let heartbeat_at_t1 = SendStamp::new(t1, 1);
        let replication_at_t2 = SendStamp::new(t2, 0);

        assert!(
            heartbeat_at_t1 > replication_at_t1,
            "a heartbeat round breaks a tie in time"
        );
        assert!(replication_at_t2 > heartbeat_at_t1, "a later time wins over any round");
    }
}
