use std::fmt::Debug;
use std::io;

use openraft_macros::add_async_trait;
use openraft_macros::since;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::testing::log::StoreBuilder;

/// Application fixtures for the optional [`ApplicationSuite`](super::ApplicationSuite).
///
/// Supply a nonempty workload with fixed log IDs and independently defined responses and state.
/// Include dependent commands whose result changes when `apply()` call boundaries are observed.
#[add_async_trait]
#[since(version = "0.10.1")]
pub trait ApplicationStoreBuilder<C, LS, SM, G = ()>: StoreBuilder<C, LS, SM, G>
where
    C: RaftTypeConfig,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    /// Complete logical application state, excluding storage layout and cached metadata.
    #[since(version = "0.10.1")]
    type State: Debug + Eq + OptionalSend;

    /// Return a fresh copy of the workload for each grouping.
    #[since(version = "0.10.1")]
    fn entries(&self) -> Vec<C::Entry>;

    /// Return the independently expected response for every workload entry.
    #[since(version = "0.10.1")]
    fn expected_responses(&self) -> Vec<C::R>;

    /// Return the independently expected final logical state.
    #[since(version = "0.10.1")]
    fn expected_state(&self) -> Self::State;

    /// Read the complete logical application state.
    #[since(version = "0.10.1")]
    async fn read_state(&self, state_machine: &mut SM) -> Result<Self::State, io::Error>;
}
