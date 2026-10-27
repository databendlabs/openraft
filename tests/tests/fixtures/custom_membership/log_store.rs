use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::ops::RangeBounds;
use std::sync::Arc;
use std::sync::Mutex;

use openraft::LogIdOptionExt;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::RaftLogReader;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::VoteOf;
use openraft::entry::RaftEntry;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;

use crate::fixtures::custom_membership::test_cluster::TestConfig;

#[derive(Clone, Debug, Default)]
pub struct LogStore<C>(Arc<Mutex<LogStoreInner<C>>>)
where C: TestConfig;

#[derive(Debug, Default)]
pub struct LogStoreInner<C>
where C: TestConfig
{
    log: BTreeMap<u64, EntryOf<C>>,
    vote: Option<VoteOf<C>>,
}

impl<C> RaftLogReader<C> for LogStore<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    async fn try_get_log_entries<RB>(&mut self, range: RB) -> Result<Vec<EntryOf<C>>, io::Error>
    where RB: RangeBounds<u64> + Clone + fmt::Debug + OptionalSend {
        let inner = self.0.lock().unwrap();
        let range_entries = inner.log.range(range);
        let entries = range_entries.map(|(_, entry)| entry.clone()).collect();
        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<C>>, io::Error> {
        let inner = self.0.lock().unwrap();
        Ok(inner.vote.clone())
    }
}

impl<C> RaftLogStorage<C> for LogStore<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<C>, io::Error> {
        let inner = self.0.lock().unwrap();
        let last_entry = inner.log.last_key_value();
        let last_log_id = last_entry.map(|(_, entry)| entry.log_id());

        Ok(LogState {
            last_purged_log_id: None,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<C>) -> Result<(), io::Error> {
        let mut inner = self.0.lock().unwrap();
        inner.vote = Some(vote.clone());
        Ok(())
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<C>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<C>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut inner = self.0.lock().unwrap();
        for entry in entries {
            inner.log.insert(entry.index(), entry);
        }
        drop(inner);

        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        let start_index = last_log_id.next_index();
        let mut inner = self.0.lock().unwrap();
        inner.log.retain(|index, _| *index < start_index);
        Ok(())
    }

    async fn purge(&mut self, _log_id: LogIdOf<C>) -> Result<(), io::Error> {
        unimplemented!("the test builds no snapshot, so it purges no log")
    }
}
