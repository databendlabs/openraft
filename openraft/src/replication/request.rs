use std::fmt;

use crate::type_config::alias::LogIdOf;

/// A replication request sent by RaftCore leader state to replication stream.
#[derive(Debug)]
pub(crate) enum Replicate<C>
where C: RaftTypeConfig
{
    /// Inform replication stream to forward the committed log id to followers/learners.
    Committed(Option<LogId<C::NodeId>>),

    /// Send an empty AppendEntries RPC as heartbeat.
    Heartbeat,

    /// Send a chunk of data, e.g., logs or snapshot.
    Data(Data<C>),
}

impl<C> Replicate<C>
where C: RaftTypeConfig
{
    pub(crate) fn logs(log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Data(Data::new_logs(log_id_range))
    }

    pub(crate) fn snapshot(last_log_id: Option<LogIdOf<C>>) -> Self {
        Self::Data(Data::new_snapshot(last_log_id))
    }

    pub(crate) fn new_data(data: Data<C>) -> Self {
        Self::Data(data)
    }
}

impl<C: RaftTypeConfig> fmt::Display for Replicate<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Committed(c) => write!(f, "Committed({})", c.display()),
            Self::Heartbeat => write!(f, "Heartbeat"),
            Self::Data(d) => write!(f, "Data({})", d),
        }
    }
}

use crate::display_ext::DisplayOptionExt;
use crate::error::Fatal;
use crate::error::StreamingError;
use crate::log_id_range::LogIdRange;
use crate::raft::SnapshotResponse;
use crate::replication::callbacks::SnapshotCallback;
use crate::type_config::alias::InstantOf;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::SnapshotMeta;

/// Request to replicate a chunk of data, logs or snapshot.
///
/// It defines what data to send to a follower/learner and an id to identify who is sending this
/// data.
/// Thd data is either a series of logs or a snapshot.
pub(crate) enum Data<C>
where C: RaftTypeConfig
{
    Heartbeat,
    Logs(LogIdRange<C::NodeId>),
    Snapshot(Option<LogIdOf<C>>),
    SnapshotCallback(SnapshotCallback<C>),
}

impl<C> fmt::Debug for Data<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Heartbeat => {
                write!(f, "Data::Heartbeat")
            }
            Self::Logs(l) => f.debug_struct("Data::Logs").field("log_id_range", l).finish(),
            Self::Snapshot(s) => f.debug_struct("Data::Snapshot").field("last_log_id", s).finish(),
            Self::SnapshotCallback(resp) => f.debug_struct("Data::SnapshotCallback").field("callback", resp).finish(),
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Data<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Heartbeat => {
                write!(f, "Heartbeat")
            }
            Self::Logs(l) => {
                write!(f, "Logs{{log_id_range: {}}}", l)
            }
            Self::Snapshot(last_log_id) => {
                write!(f, "Snapshot{{last_log_id: {}}}", last_log_id.display())
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
    pub(crate) fn new_heartbeat() -> Self {
        Self::Heartbeat
    }

    pub(crate) fn new_logs(log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Logs(log_id_range)
    }

    pub(crate) fn new_snapshot(last_log_id: Option<LogIdOf<C>>) -> Self {
        Self::Snapshot(last_log_id)
    }

    pub(crate) fn new_snapshot_callback(
        start_time: InstantOf<C>,
        snapshot_meta: SnapshotMeta<C>,
        result: Result<SnapshotResponse<C>, StreamingError<C, Fatal<C>>>,
    ) -> Self {
        Self::SnapshotCallback(SnapshotCallback::new(start_time, snapshot_meta, result))
    }

    /// Return true if the data includes any payload, i.e., not a heartbeat.
    pub(crate) fn has_payload(&self) -> bool {
        match self {
            Self::Heartbeat => false,
            Self::Logs(_) => true,
            Self::Snapshot(_) => true,
            Self::SnapshotCallback(_) => true,
        }
    }
}
