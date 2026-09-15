/// Traffic policy for one runtime Leader session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaderActivity<I> {
    Active { observe_until: Option<I> },
    AwaitingQuorum { inactive_at: I },
    Inactive { next_probe_at: I },
}
