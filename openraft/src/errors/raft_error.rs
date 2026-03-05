use std::fmt::Debug;

use openraft_macros::since;
use peel_off::Peel;

use crate::RaftTypeConfig;
use crate::StorageError;
use crate::errors::Fatal;
use crate::errors::ForwardToLeader;
use crate::errors::Infallible;
use crate::try_as_ref::TryAsRef;

/// Error returned by Raft API methods.
///
/// `RaftError` wraps either a [`Fatal`] error indicating the Raft node has stopped (due to storage
/// failure, panic, or shutdown), or an API-specific error `E` (such as [`ClientWriteError`] or
/// [`LinearizableReadError`]).
///
/// # Usage
///
/// Match on the error variant to handle appropriately:
///
/// ```ignore
/// match raft.client_write(req).await {
///     Ok(resp) => { /* handle response */ },
///     Err(RaftError::APIError(e)) => {
///         // Handle API error (e.g., forward to leader)
///     }
///     Err(RaftError::Fatal(f)) => {
///         // Raft stopped - initiate shutdown
///     }
/// }
/// ```
///
/// [`ClientWriteError`]: crate::errors::ClientWriteError
/// [`LinearizableReadError`]: crate::errors::LinearizableReadError
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum RaftError<C, E = Infallible>
where C: RaftTypeConfig
{
    /// API-specific error returned by Raft API methods.
    #[error(transparent)]
    APIError(E),

    /// Fatal error indicating the Raft node has stopped.
    // Reset serde trait bound for C but not for E
    #[cfg_attr(feature = "serde", serde(bound = ""))]
    #[error(transparent)]
    Fatal(#[from] Fatal<C>),
}

impl<C> RaftError<C, Infallible>
where C: RaftTypeConfig
{
    /// Convert to a [`Fatal`] error if its `APIError` variant is [`Infallible`],
    /// otherwise panic.
    #[since(version = "0.10.0")]
    pub fn unwrap_fatal(self) -> Fatal<C> {
        self.into_fatal().unwrap()
    }
}

impl<C, E> RaftError<C, E>
where
    C: RaftTypeConfig,
    E: Debug,
{
    /// Return a reference to Self::APIError.
    pub fn api_error(&self) -> Option<&E> {
        match self {
            RaftError::APIError(e) => Some(e),
            RaftError::Fatal(_) => None,
        }
    }

    /// Try to convert self to APIError.
    pub fn into_api_error(self) -> Option<E> {
        match self {
            RaftError::APIError(e) => Some(e),
            RaftError::Fatal(_) => None,
        }
    }

    /// Return a reference to Self::Fatal.
    pub fn fatal(&self) -> Option<&Fatal<C>> {
        match self {
            RaftError::APIError(_) => None,
            RaftError::Fatal(f) => Some(f),
        }
    }

    /// Try to convert self to Fatal error.
    pub fn into_fatal(self) -> Option<Fatal<C>> {
        match self {
            RaftError::APIError(_) => None,
            RaftError::Fatal(f) => Some(f),
        }
    }

    /// Return a reference to ForwardToLeader if Self::APIError contains it.
    pub fn forward_to_leader(&self) -> Option<&ForwardToLeader<C>>
    where E: TryAsRef<ForwardToLeader<C>> {
        match self {
            RaftError::APIError(api_err) => api_err.try_as_ref(),
            RaftError::Fatal(_) => None,
        }
    }

    /// Try to convert self to ForwardToLeader error if APIError is a ForwardToLeader error.
    pub fn into_forward_to_leader(self) -> Option<ForwardToLeader<C>>
    where E: TryInto<ForwardToLeader<C>> {
        match self {
            RaftError::APIError(api_err) => api_err.try_into().ok(),
            RaftError::Fatal(_) => None,
        }
    }
}

impl<C, E> TryAsRef<ForwardToLeader<C>> for RaftError<C, E>
where
    C: RaftTypeConfig,
    E: Debug + TryAsRef<ForwardToLeader<C>>,
{
    fn try_as_ref(&self) -> Option<&ForwardToLeader<C>> {
        self.forward_to_leader()
    }
}

impl<C, E> From<StorageError<C>> for RaftError<C, E>
where C: RaftTypeConfig
{
    fn from(se: StorageError<C>) -> Self {
        RaftError::Fatal(Fatal::from(se))
    }
}

/// Peel off `Fatal`, leaving the API error `E` as the residual.
impl<C, E> Peel for RaftError<C, E>
where
    C: RaftTypeConfig,
    E: Debug,
{
    type Peeled = Fatal<C>;
    type Residual = E;

    fn peel(self) -> Result<E, Fatal<C>> {
        match self {
            RaftError::APIError(e) => Ok(e),
            RaftError::Fatal(f) => Err(f),
        }
    }
}
