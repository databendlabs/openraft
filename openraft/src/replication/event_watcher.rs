use futures::FutureExt;

use crate::RaftTypeConfig;
use crate::async_runtime::watch::RecvError;
use crate::async_runtime::watch::WatchReceiver;
use crate::replication::request::Data;
use crate::replication::request::Replicate;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::WatchReceiverOf;

#[derive(Clone)]
pub(crate) struct EventWatcher<C>
where C: RaftTypeConfig
{
    pub(crate) entries_rx: WatchReceiverOf<C, Data<C>>,
    pub(crate) committed_rx: WatchReceiverOf<C, Option<LogIdOf<C>>>,
}

impl<C> EventWatcher<C>
where C: RaftTypeConfig
{
    pub(crate) async fn recv(&mut self) -> Result<Replicate<C>, RecvError> {
        let entries = self.entries_rx.changed();
        let committed = self.committed_rx.changed();

        futures::select! {
            entries_res = entries.fuse() => {
                entries_res?;

                let data = self.entries_rx.borrow_watched().clone();
                Ok(Replicate::Data {data})
            }
            committed_res = committed.fuse() => {
                committed_res?;

                let committed = self.committed_rx.borrow_watched().clone();
                Ok(Replicate::Committed {committed})
            }
        }
    }
}
