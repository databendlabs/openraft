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
    pub(crate) fn logs(id: RequestId, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Data(Data::new_logs(id, log_id_range))
    }

    pub(crate) fn snapshot(id: RequestId, last_log_id: Option<LogIdOf<C>>) -> Self {
        Self::Data(Data::new_snapshot(id, last_log_id))
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

impl<C> MessageSummary<Replicate<C>> for Replicate<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        self.to_string()
    }
}

use crate::display_ext::DisplayOptionExt;
use crate::error::StreamingError;
use crate::log_id_range::LogIdRange;
use crate::raft::SnapshotResponse;
use crate::replication::callbacks::SnapshotCallback;
use crate::replication::request_id::RequestId;
use crate::type_config::alias::InstantOf;
use crate::LogId;
use crate::MessageSummary;
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
    Logs(DataWithId<LogIdRange<C::NodeId>>),
    Snapshot(DataWithId<Option<LogIdOf<C>>>),
    SnapshotCallback(DataWithId<SnapshotCallback<C>>),
}

impl<C> fmt::Debug for Data<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Heartbeat => {
                write!(f, "Data::Heartbeat")
            }
            Self::Logs(l) => f
                .debug_struct("Data::Logs")
                .field("request_id", &l.request_id())
                .field("log_id_range", &l.data)
                .finish(),
            Self::Snapshot(s) => f.debug_struct("Data::Snapshot").field("request_id", &s.request_id()).finish(),
            Self::SnapshotCallback(resp) => f
                .debug_struct("Data::SnapshotCallback")
                .field("request_id", &resp.request_id())
                .field("callback", &resp.data)
                .finish(),
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
                write!(f, "Logs{{request_id: {}, log_id_range: {}}}", l.request_id, l.data)
            }
            Self::Snapshot(s) => {
                write!(f, "Snapshot{{request_id: {}}}", s.request_id)
            }
            Self::SnapshotCallback(l) => {
                write!(
                    f,
                    "SnapshotCallback{{request_id: {}, callback: {}}}",
                    l.request_id, l.data
                )
            }
        }
    }
}

impl<C> MessageSummary<Data<C>> for Data<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        self.to_string()
    }
}

impl<C> Data<C>
where C: RaftTypeConfig
{
    pub(crate) fn new_heartbeat() -> Self {
        Self::Heartbeat
    }

    pub(crate) fn new_logs(request_id: RequestId, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Logs(DataWithId::new(request_id, log_id_range))
    }

    pub(crate) fn new_snapshot(request_id: RequestId, last_log_id: Option<LogIdOf<C>>) -> Self {
        Self::Snapshot(DataWithId::new(request_id, last_log_id))
    }

    pub(crate) fn new_snapshot_callback(
        request_id: RequestId,
        start_time: InstantOf<C>,
        snapshot_meta: SnapshotMeta<C::NodeId, C::Node>,
        result: Result<SnapshotResponse<C::NodeId>, StreamingError<C>>,
    ) -> Self {
        Self::SnapshotCallback(DataWithId::new(
            request_id,
            SnapshotCallback::new(start_time, snapshot_meta, result),
        ))
    }

    pub(crate) fn request_id(&self) -> RequestId {
        match self {
            Self::Heartbeat => RequestId::new_heartbeat(),
            Self::Logs(l) => l.request_id(),
            Self::Snapshot(s) => s.request_id(),
            Self::SnapshotCallback(r) => r.request_id(),
        }
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

#[derive(Clone)]
pub(crate) struct DataWithId<T> {
    /// The id of this replication request.
    request_id: RequestId,
    data: T,
}

impl<T> DataWithId<T> {
    pub(crate) fn new(request_id: RequestId, data: T) -> Self {
        Self { request_id, data }
    }

    pub(crate) fn request_id(&self) -> RequestId {
        self.request_id
    }

    pub(crate) fn data(&self) -> &T {
        &self.data
    }

    pub(crate) fn into_data(self) -> T {
        self.data
    }
}
