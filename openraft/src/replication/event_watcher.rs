use crate::RaftTypeConfig;
use crate::raft_state::IOId;
use crate::replication::replicate::Replicate;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::WatchReceiverOf;

#[derive(Clone)]
pub(crate) struct EventWatcher<C>
where C: RaftTypeConfig
{
    pub(crate) replicate_rx: WatchReceiverOf<C, Replicate<C>>,
    pub(crate) committed_rx: WatchReceiverOf<C, Option<LogIdOf<C>>>,

    pub(crate) io_accepted_rx: WatchReceiverOf<C, IOId<C>>,
    pub(crate) io_submitted_rx: WatchReceiverOf<C, IOId<C>>,
}
