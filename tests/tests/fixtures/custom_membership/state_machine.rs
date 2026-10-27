use std::io;
use std::sync::Arc;
use std::sync::Mutex;

use futures::Stream;
use futures::TryStreamExt;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::entry::RaftEntry;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;

use crate::fixtures::custom_membership::test_cluster::TestConfig;

/// A state machine that keeps every entry it applies.
#[derive(Clone, Debug, Default)]
pub struct StateMachine<C>(Arc<Mutex<StateMachineInner<C>>>)
where C: TestConfig;

#[derive(Debug, Default)]
pub struct StateMachineInner<C>
where C: TestConfig
{
    applied: Vec<EntryOf<C>>,
    membership: StoredMembershipOf<C>,
}

impl<C> StateMachine<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    pub(super) fn applied(&self) -> Vec<EntryOf<C>> {
        let inner = self.0.lock().unwrap();
        inner.applied.clone()
    }
}

impl<C> RaftSnapshotBuilder<C> for StateMachine<C>
where C: TestConfig
{
    type SnapshotData = ();

    async fn build_snapshot(&mut self) -> Result<SnapshotOf<C, ()>, io::Error> {
        unimplemented!("the test builds no snapshot")
    }
}

impl<C> RaftStateMachine<C> for StateMachine<C>
where C: TestConfig
{
    type SnapshotData = ();
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error> {
        let inner = self.0.lock().unwrap();
        let last_entry = inner.applied.last();
        let last_applied = last_entry.map(|entry| entry.log_id());
        Ok((last_applied, inner.membership.clone()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        while let Some((entry, responder)) = entries.try_next().await? {
            {
                let mut inner = self.0.lock().unwrap();
                if let Some(membership) = entry.get_membership() {
                    let log_id = entry.log_id();
                    inner.membership = StoredMembershipOf::<C>::new(Some(log_id), membership);
                }
                inner.applied.push(entry);
            }

            if let Some(responder) = responder {
                responder.send(());
            }
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn install_snapshot(&mut self, _meta: &SnapshotMetaOf<C>, _snapshot: ()) -> Result<(), io::Error> {
        unimplemented!("the test sends no snapshot")
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, ()>>, io::Error> {
        Ok(None)
    }
}
