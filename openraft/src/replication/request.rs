use std::fmt;

/// A replication request sent by RaftCore leader state to replication stream.
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
    pub(crate) fn logs(id: Option<u64>, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Data(Data::new_logs(id, log_id_range))
    }

    pub(crate) fn snapshot(id: Option<u64>, snapshot_rx: oneshot::Receiver<Option<Snapshot<C>>>) -> Self {
        Self::Data(Data::new_snapshot(id, snapshot_rx))
    }
}

impl<C> MessageSummary<Replicate<C>> for Replicate<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match self {
            Replicate::Committed(c) => {
                format!("Replicate::Committed: {:?}", c)
            }
            Replicate::Heartbeat => "Replicate::Heartbeat".to_string(),
            Replicate::Data(d) => {
                format!("Replicate::Data({})", d.summary())
            }
        }
    }
}

use tokio::sync::oneshot;

use crate::display_ext::DisplayOptionExt;
use crate::log_id_range::LogIdRange;
use crate::LogId;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::Snapshot;

/// Request to replicate a chunk of data, logs or snapshot.
///
/// It defines what data to send to a follower/learner and an id to identify who is sending this
/// data.
/// Thd data is either a series of logs or a snapshot.
pub(crate) enum Data<C>
where C: RaftTypeConfig
{
    Logs(DataWithId<LogIdRange<C::NodeId>>),
    Snapshot(DataWithId<oneshot::Receiver<Option<Snapshot<C>>>>),
}

impl<C> fmt::Debug for Data<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Logs(l) => f
                .debug_struct("Data::Logs")
                .field("request_id", &l.request_id())
                .field("log_id_range", &l.data)
                .finish(),
            Self::Snapshot(s) => f.debug_struct("Data::Snapshot").field("request_id", &s.request_id()).finish(),
        }
    }
}

impl<C: RaftTypeConfig> fmt::Display for Data<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Logs(l) => {
                write!(
                    f,
                    "Logs{{request_id: {}, log_id_range: {}}}",
                    l.request_id.display(),
                    l.data
                )
            }
            Self::Snapshot(s) => {
                write!(f, "Snapshot{{request_id: {}}}", s.request_id.display())
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
    pub(crate) fn new_logs(request_id: Option<u64>, log_id_range: LogIdRange<C::NodeId>) -> Self {
        Self::Logs(DataWithId::new(request_id, log_id_range))
    }

    pub(crate) fn new_snapshot(request_id: Option<u64>, snapshot_rx: oneshot::Receiver<Option<Snapshot<C>>>) -> Self {
        Self::Snapshot(DataWithId::new(request_id, snapshot_rx))
    }

    pub(crate) fn request_id(&self) -> Option<u64> {
        match self {
            Self::Logs(l) => l.request_id(),
            Self::Snapshot(s) => s.request_id(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct DataWithId<T> {
    /// The id of this replication request.
    ///
    /// A replication request without an id does not need to send a reply to the caller.
    request_id: Option<u64>,
    data: T,
}

impl<T> DataWithId<T> {
    pub(crate) fn new(request_id: Option<u64>, data: T) -> Self {
        Self { request_id, data }
    }

    pub(crate) fn request_id(&self) -> Option<u64> {
        self.request_id
    }

    pub(crate) fn data(&self) -> &T {
        &self.data
    }

    pub(crate) fn into_data(self) -> T {
        self.data
    }
}
