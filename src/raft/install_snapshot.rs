use actix::prelude::*;
use log::{warn};

use crate::{
    AppError,
    network::RaftNetwork,
    messages::{InstallSnapshotRequest, InstallSnapshotResponse},
    raft::{RaftState, Raft},
    storage::RaftStorage,
};

impl<E: AppError, N: RaftNetwork<E>, S: RaftStorage<E>> Handler<InstallSnapshotRequest> for Raft<E, N, S> {
    type Result = ResponseActFuture<Self, InstallSnapshotResponse, ()>;

    /// Invoked by leader to send chunks of a snapshot to a follower (§7).
    ///
    /// Leaders always send chunks in order. It is important to note that, according to the Raft spec,
    /// a log may only have one snapshot at any time. As snapshot contents are application specific,
    /// the Raft log will only store a pointer to the snapshot file along with the index & term.
    ///
    /// Receiver implementation:
    /// 1. Reply immediately if `term` is less than receiver's current `term`.
    /// 2. Create a new snapshot file if snapshot received is the first chunk
    ///    of the sanpshot (offset is 0).
    /// 3. Write data into snapshot file at given offset.
    /// 4. Reply and wait for more data chunks if `done` is `false`.
    /// 5. Save snapshot file, discard any existing or partial snapshot with a smaller index.
    /// 6. If existing log entry has same index and term as snapshot’s last included entry,
    ///    retain log entries following it and reply.
    /// 7. Discard the entire log.
    /// 8. Reset state machine using snapshot contents and load snapshot’s cluster configuration.
    fn handle(&mut self, msg: InstallSnapshotRequest, _ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if let &RaftState::Initializing = &self.state {
            warn!("Received Raft RPC before initialization was complete.");
            return Box::new(fut::err(()));
        }

        // Don't interact with non-cluster members.
        if !self.members.contains(&msg.leader_id) {
            return Box::new(fut::err(()));
        }

        // Unpack the given message and pass to the appropriate handler.
        // TODO: this needs to be implemented.
        // TODO: update election timeout.
        Box::new(fut::err(()))
    }
}
