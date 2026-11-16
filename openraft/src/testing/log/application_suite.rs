use std::fmt::Debug;
use std::io;
use std::marker::PhantomData;
use std::time::Duration;

use futures_util::stream;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::entry::RaftEntry;
use crate::impls::ProgressResponder;
use crate::raft::ClientWriteResult;
use crate::raft::responder::core_responder::CoreResponder;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::storage::v2::entry_responder::EntryResponderBuilder;
use crate::testing::log::ApplicationStoreBuilder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Optional checks for application state and responses across `apply()` stream groupings.
///
/// Run [`super::Suite`] as well. Transport, reopen, and crash checks stay with the adapters that
/// own those operations.
#[since(version = "0.10.1")]
pub struct ApplicationSuite<C, LS, SM, B, G = ()> {
    _types: PhantomData<(C, LS, SM, B, G)>,
}

impl<C, LS, SM, B, G> ApplicationSuite<C, LS, SM, B, G>
where
    C: RaftTypeConfig,
    C::R: Debug + Eq,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
    B: ApplicationStoreBuilder<C, LS, SM, G>,
{
    /// Apply the same workload together, individually, and in uneven groups.
    #[since(version = "0.10.1")]
    pub async fn test_all(builder: &B) -> Result<(), io::Error> {
        Self::apply_grouping(builder, Grouping::Together).await?;
        Self::apply_grouping(builder, Grouping::Individually).await?;
        Self::apply_grouping(builder, Grouping::Uneven).await
    }

    async fn apply_grouping(builder: &B, grouping: Grouping) -> Result<(), io::Error> {
        let (_guard, _logs, mut state_machine) = builder.build().await.map_err(storage_error)?;
        let entries = builder.entries();
        assert!(!entries.is_empty(), "application workload must contain entries");

        let expected_applied = expected_applied::<C>(&entries);
        let batches = grouping.split::<C>(entries);
        let mut responses = Vec::new();
        for batch in batches {
            let batch_responses = apply::<C, SM>(&mut state_machine, batch).await?;
            responses.extend(batch_responses);
        }

        let expected_responses = builder.expected_responses();
        assert_eq!(expected_responses, responses);
        let expected_state = builder.expected_state();
        let state = builder.read_state(&mut state_machine).await?;
        assert_eq!(expected_state, state);
        let applied = state_machine.applied_state().await?;
        assert_eq!(expected_applied, applied);
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Grouping {
    Together,
    Individually,
    Uneven,
}

impl Grouping {
    fn split<C>(self, entries: Vec<C::Entry>) -> Vec<Vec<C::Entry>>
    where C: RaftTypeConfig {
        match self {
            Self::Together => vec![entries],
            Self::Individually => entries.into_iter().map(|entry| vec![entry]).collect(),
            Self::Uneven => split_uneven(entries),
        }
    }
}

fn split_uneven<E>(entries: Vec<E>) -> Vec<Vec<E>> {
    let mut entries = entries.into_iter();
    let mut batches = Vec::new();
    for size in [1, 3, 2].into_iter().cycle() {
        let batch: Vec<E> = entries.by_ref().take(size).collect();
        if batch.is_empty() {
            break;
        }
        batches.push(batch);
    }
    batches
}

fn expected_applied<C>(entries: &[C::Entry]) -> (Option<LogIdOf<C>>, StoredMembershipOf<C>)
where C: RaftTypeConfig {
    let mut membership = StoredMembershipOf::<C>::default();
    for entry in entries {
        let Some(value) = entry.get_membership() else {
            continue;
        };
        membership = StoredMembershipOf::<C>::new(Some(entry.log_id()), value);
    }
    let last_log_id = entries.last().map(RaftEntry::log_id);
    (last_log_id, membership)
}

fn storage_error<C>(error: crate::StorageError<C>) -> io::Error
where C: RaftTypeConfig {
    io::Error::other(error.to_string())
}

async fn apply<C, SM>(state_machine: &mut SM, entries: Vec<C::Entry>) -> Result<Vec<C::R>, io::Error>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    let mut receivers = Vec::new();
    let mut items = Vec::new();
    for entry in entries {
        let (responder, receiver) = ProgressResponder::<C, ClientWriteResult<C>>::complete_only();
        let responder = CoreResponder::Progress(responder);
        let builder = EntryResponderBuilder {
            entry,
            responder: Some(responder),
        };
        let item = builder.into_parts();
        items.push(Ok(item));
        receivers.push(receiver);
    }

    state_machine.apply(stream::iter(items)).await?;

    let mut responses = Vec::new();
    for receiver in receivers {
        let received = C::timeout(RESPONSE_TIMEOUT, receiver).await;
        let received = received.map_err(|_| io::Error::other("apply response timed out"))?;
        let received = received.map_err(|_| io::Error::other("apply response channel closed"))?;
        let response = received.map_err(|error| io::Error::other(error.to_string()))?;
        responses.push(response.data);
    }
    Ok(responses)
}
