/// Traffic policy for one runtime Leader session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaderActivity<I> {
    Active {
        /// Earliest time to check whether the current quorum evidence is still valid.
        next_check_at: I,
    },
    Inactive {
        /// Earliest time to submit the next quorum recovery probe round.
        next_probe_at: I,
    },
}
