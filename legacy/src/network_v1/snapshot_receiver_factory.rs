use std::io;

use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::storage::RaftStateMachine;
use openraft_macros::add_async_trait;
use openraft_macros::since;

/// Creates writable handles for receiving snapshots through the v1 chunk protocol.
///
/// This trait replaces the former `RaftStateMachine::begin_receiving_snapshot()` capability,
/// keeping legacy snapshot receiver construction out of the core state-machine API. Implement it
/// only for state machines that receive snapshots through the legacy v1 chunk protocol.
///
/// Before, receiver construction was part of every state-machine implementation:
///
/// ```ignore
/// impl RaftStateMachine<TypeConfig> for StateMachine {
///     type SnapshotData = Cursor<Vec<u8>>;
///     type SnapshotReceiver = Cursor<Vec<u8>>;
///
///     async fn begin_receiving_snapshot(&mut self) -> io::Result<Self::SnapshotReceiver> {
///         Ok(Cursor::new(Vec::new()))
///     }
///
///     // Other RaftStateMachine items...
/// }
/// ```
///
/// Now, only users of the v1 chunk protocol add a separate capability implementation:
///
/// ```ignore
/// impl RaftStateMachine<TypeConfig> for StateMachine {
///     type SnapshotData = Cursor<Vec<u8>>;
///
///     // Other RaftStateMachine items...
/// }
///
/// impl SnapshotReceiverFactory<TypeConfig> for StateMachine {
///     type SnapshotReceiver = Cursor<Vec<u8>>;
///
///     async fn begin_receiving_snapshot(&mut self) -> io::Result<Self::SnapshotReceiver> {
///         Ok(Cursor::new(Vec::new()))
///     }
/// }
/// ```
#[add_async_trait]
#[since(version = "0.10.0")]
pub trait SnapshotReceiverFactory<C>: RaftStateMachine<C>
where C: RaftTypeConfig
{
    /// Write target for receiving a snapshot from the leader.
    #[since(version = "0.10.0", change = "moved from RaftStateMachine")]
    type SnapshotReceiver: OptionalSend + 'static;

    /// Create a new blank snapshot receiver.
    #[since(version = "0.10.0", change = "moved from RaftStateMachine")]
    async fn begin_receiving_snapshot(&mut self) -> Result<Self::SnapshotReceiver, io::Error>;
}
