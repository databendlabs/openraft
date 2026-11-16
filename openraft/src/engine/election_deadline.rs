use std::fmt;
use std::time::Duration;

use display_more::DisplayOptionExt;

use crate::Instant;
use crate::display_ext::DisplayInstantExt;

/// When the election timing permits this node to start its next election.
///
/// It covers timing only. The caller still checks the server state, campaign eligibility, and
/// runtime flags before it starts an election.
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
pub(crate) struct ElectionDeadline<I>
where I: Instant
{
    /// An election is due only strictly after this instant, when the vote lease extended by the
    /// election timeout has expired. See
    /// [`Leased::expires_at()`](crate::utime::Leased::expires_at).
    ///
    /// `None` imposes no lease wait: the vote was never updated, or this node is the single
    /// campaigner.
    pub(crate) lease_expires_at: Option<I>,

    /// An election is due only at or after this instant, when the in-flight Pre-Vote round has
    /// been pending for an election timeout.
    ///
    /// `None` when the next election skips Pre-Vote or no Pre-Vote round is in flight.
    pub(crate) pre_vote_retry_at: Option<I>,
}

impl<I> ElectionDeadline<I>
where I: Instant
{
    /// Whether the election timing permits starting an election at `now`.
    pub(crate) fn is_due(&self, now: I) -> bool {
        let lease_expired = self.lease_expires_at.is_none_or(|t| now > t);
        let retry_allowed = self.pre_vote_retry_at.is_none_or(|t| now >= t);
        lease_expired && retry_allowed
    }

    /// Return the earliest instant at which [`Self::is_due()`] holds, or `None` when it holds at
    /// every instant.
    ///
    /// The lease wait ends one nanosecond after `lease_expires_at`, the smallest step past its
    /// strict boundary.
    pub(crate) fn earliest_due_at(&self) -> Option<I> {
        let after_lease = self.lease_expires_at.map(|t| t + Duration::from_nanos(1));
        after_lease.max(self.pre_vote_retry_at)
    }
}

impl<I> fmt::Display for ElectionDeadline<I>
where I: Instant
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ElectionDeadline{{lease_expires_at: {}, pre_vote_retry_at: {}}}",
            self.lease_expires_at.as_ref().map(|t| t.display()).display(),
            self.pre_vote_retry_at.as_ref().map(|t| t.display()).display(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::ElectionDeadline;

    const NANO: Duration = Duration::from_nanos(1);

    #[test]
    fn test_no_wait_is_due_at_once() {
        let deadline = ElectionDeadline::<Instant> {
            lease_expires_at: None,
            pre_vote_retry_at: None,
        };
        assert!(deadline.is_due(Instant::now()));
    }

    #[test]
    fn test_lease_is_due_strictly_after_its_boundary() {
        let boundary = Instant::now();
        let deadline = ElectionDeadline {
            lease_expires_at: Some(boundary),
            pre_vote_retry_at: None,
        };

        assert!(!deadline.is_due(boundary - NANO));
        assert!(!deadline.is_due(boundary), "the lease is still valid at its boundary");
        assert!(deadline.is_due(boundary + NANO));
    }

    #[test]
    fn test_pre_vote_retry_is_due_at_its_boundary() {
        let retry_at = Instant::now();
        let deadline = ElectionDeadline {
            lease_expires_at: None,
            pre_vote_retry_at: Some(retry_at),
        };

        assert!(!deadline.is_due(retry_at - NANO));
        assert!(deadline.is_due(retry_at));
    }

    #[test]
    fn test_both_waits_must_pass() {
        let t0 = Instant::now();
        let early = t0 + Duration::from_millis(10);
        let late = t0 + Duration::from_millis(20);

        let lease_first = ElectionDeadline {
            lease_expires_at: Some(early),
            pre_vote_retry_at: Some(late),
        };
        assert!(!lease_first.is_due(early + NANO), "the Pre-Vote retry is not due yet");
        assert!(lease_first.is_due(late));

        let retry_first = ElectionDeadline {
            lease_expires_at: Some(late),
            pre_vote_retry_at: Some(early),
        };
        assert!(!retry_first.is_due(late), "the lease has not expired yet");
        assert!(retry_first.is_due(late + NANO));
    }

    #[test]
    fn test_earliest_due_at_is_the_first_due_instant() {
        let t0 = Instant::now();
        let early = t0 + Duration::from_millis(10);
        let late = t0 + Duration::from_millis(20);

        let no_wait = ElectionDeadline::<Instant> {
            lease_expires_at: None,
            pre_vote_retry_at: None,
        };
        assert_eq!(None, no_wait.earliest_due_at());

        let cases = [
            (Some(early), None, early + NANO),
            (None, Some(early), early),
            (Some(early), Some(late), late),
            (Some(late), Some(early), late + NANO),
        ];

        for (lease_expires_at, pre_vote_retry_at, want) in cases {
            let deadline = ElectionDeadline {
                lease_expires_at,
                pre_vote_retry_at,
            };
            assert_eq!(Some(want), deadline.earliest_due_at(), "{deadline:?}");
            assert!(deadline.is_due(want), "{deadline:?}");
            assert!(!deadline.is_due(want - NANO), "{deadline:?}");
        }
    }
}
