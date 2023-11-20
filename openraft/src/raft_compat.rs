//! Compatibility layer for `Raft` with old type parameters.

use crate::storage::RaftLogStorage;
use crate::storage::RaftStateMachine;
use crate::RaftNetworkFactory;
use crate::RaftTypeConfig;
use crate::StorageTypeConfig;
use std::marker::PhantomData;

/// Default type for storage configuration for compatibility.
///
/// This type implements [`StorageTypeConfig<C>`] with the supplied types for network,
/// log storage and state machine.
pub struct DefaultStorageConfig<C, N, LS, SM>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    _phantom: PhantomData<(C, N, LS, SM)>,
}

impl<C, N, LS, SM> StorageTypeConfig<C> for DefaultStorageConfig<C, N, LS, SM>
where
    C: RaftTypeConfig,
    N: RaftNetworkFactory<C>,
    LS: RaftLogStorage<C>,
    SM: RaftStateMachine<C>,
{
    type NetworkFactory = N;
    type LogStorage = LS;
    type StateMachine = SM;
}

/// Type alias to forward to the new `Raft` implementation.
pub type Raft<C, N, LS, SM> = crate::raft::Raft<C, DefaultStorageConfig<C, N, LS, SM>>;
