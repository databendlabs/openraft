use std::fmt;

use crate::type_config::alias::LogIdOf;

/// A replication request sent by RaftCore leader state to replication stream.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
pub(crate) enum Replicate<C>
where C: RaftTypeConfig
{
    /// Inform the replication stream to forward the committed log id to followers/learners.
    Committed(Option<LogIdOf<C>>),

    /// Send a chunk of data, e.g., logs or snapshot.
    Data(Data<C>),
}

impl<C> Replicate<C>
where C: RaftTypeConfig
{
    pub(crate) fn logs(log_id_range: LogIdRange<C>) -> Self {
        Self::Data(Data::new_logs(log_id_range))
    }

    pub(crate) fn snapshot() -> Self {
        Self::Data(Data::new_snapshot())
    }

    pub(crate) fn new_data(data: Data<C>) -> Self {
        Self::Data(data)
    }
}

impl<C: RaftTypeConfig> fmt::Display for Replicate<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Committed(c) => write!(f, "Committed({})", c.display()),
            Self::Data(d) => write!(f, "Data({})", d),
        }
    }
}

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::error::StreamingError;
use crate::log_id_range::LogIdRange;
use crate::raft::SnapshotResponse;
use crate::replication::callbacks::SnapshotCallback;
use crate::storage::SnapshotMeta;
use crate::type_config::alias::InstantOf;

/// Request to replicate a chunk of data, logs or snapshot.
///
/// It defines what data to send to a follower/learner and an id to identify who is sending this
/// data.
/// The data is either a series of logs or a snapshot.
#[derive(PartialEq, Eq)]
pub(crate) enum Data<C>
where C: RaftTypeConfig
{
    Committed,
    Logs(LogIdRange<C>),
    Snapshot,
    SnapshotCallback(SnapshotCallback<C>),
}

impl<C> fmt::Debug for Data<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Committed => {
                write!(f, "Data::Committed")
            }
            Self::Logs(l) => f.debug_struct("Data::Logs").field("log_id_range", l).finish(),
            Self::Snapshot => f.debug_struct("Data::Snapshot").finish(),
            Self::SnapshotCallback(resp) => f.debug_struct("Data::SnapshotCallback").field("callback", resp).finish(),
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Data<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Committed => {
                write!(f, "Committed")
            }
            Self::Logs(l) => {
                write!(f, "Logs{{log_id_range: {}}}", l)
            }
            Self::Snapshot => {
                write!(f, "Snapshot")
            }
            Self::SnapshotCallback(l) => {
                write!(f, "SnapshotCallback{{callback: {}}}", l)
            }
        }
    }
}

impl<C> Data<C>
where C: RaftTypeConfig
{
    pub(crate) fn new_committed() -> Self {
        Self::Committed
    }

    pub(crate) fn new_logs(log_id_range: LogIdRange<C>) -> Self {
        Self::Logs(log_id_range)
    }

    pub(crate) fn new_snapshot() -> Self {
        Self::Snapshot
    }

    pub(crate) fn new_snapshot_callback(
        start_time: InstantOf<C>,
        snapshot_meta: SnapshotMeta<C>,
        result: Result<SnapshotResponse<C>, StreamingError<C>>,
    ) -> Self {
        Self::SnapshotCallback(SnapshotCallback::new(start_time, snapshot_meta, result))
    }

    /// Return true if the data includes any payload, i.e., not a heartbeat.
    pub(crate) fn has_payload(&self) -> bool {
        match self {
            Self::Committed => false,
            Self::Logs(_) => true,
            Self::Snapshot => true,
            Self::SnapshotCallback(_) => true,
        }
    }
}
