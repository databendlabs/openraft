//! Public Raft interface and data types.

use std::sync::Arc;

use serde::{Serialize, Deserialize};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::{AppData, AppDataResponse, AppError, NodeId, RaftNetwork, RaftStorage};
use crate::config::Config;
use crate::error::{ClientError, InitWithConfigError, ProposeConfigChangeError, RaftError, RaftResult};
use crate::metrics::RaftMetrics;
use crate::core::RaftCore;

pub(crate) type TxAppendEntriesResponse<E> = oneshot::Sender<RaftResult<AppendEntriesResponse, E>>;
pub(crate) type TxVoteResponse<E> = oneshot::Sender<RaftResult<VoteResponse, E>>;
pub(crate) type TxInstallSnapshotResponse<E> = oneshot::Sender<RaftResult<InstallSnapshotResponse, E>>;
pub(crate) type TxClientResponse<D, R, E> = oneshot::Sender<Result<ClientResponse<R>, ClientError<D, E>>>;
pub(crate) type TxInitResponse<E> = oneshot::Sender<Result<(), InitWithConfigError<E>>>;
pub(crate) type TxProposeResponse<E> = oneshot::Sender<Result<(), ProposeConfigChangeError<E>>>;

pub(crate) type TxChanAppendEntries<D, E> = mpsc::UnboundedSender<(AppendEntriesRequest<D>, TxAppendEntriesResponse<E>)>;
pub(crate) type TxChanVote<E> = mpsc::UnboundedSender<(VoteRequest, TxVoteResponse<E>)>;
pub(crate) type TxChanInstallSnapshot<E> = mpsc::UnboundedSender<(InstallSnapshotRequest, TxInstallSnapshotResponse<E>)>;
pub(crate) type TxChanClient<D, R, E> = mpsc::UnboundedSender<(ClientRequest<D>, TxClientResponse<D, R, E>)>;
pub(crate) type TxChanInit<E> = mpsc::UnboundedSender<(InitWithConfig, TxInitResponse<E>)>;
pub(crate) type TxChanPropose<E> = mpsc::UnboundedSender<(ProposeConfigChange, TxProposeResponse<E>)>;

pub(crate) type RxChanAppendEntries<D, E> = mpsc::UnboundedReceiver<(AppendEntriesRequest<D>, TxAppendEntriesResponse<E>)>;
pub(crate) type RxChanVote<E> = mpsc::UnboundedReceiver<(VoteRequest, TxVoteResponse<E>)>;
pub(crate) type RxChanInstallSnapshot<E> = mpsc::UnboundedReceiver<(InstallSnapshotRequest, TxInstallSnapshotResponse<E>)>;
pub(crate) type RxChanClient<D, R, E> = mpsc::UnboundedReceiver<(ClientRequest<D>, TxClientResponse<D, R, E>)>;
pub(crate) type RxChanInit<E> = mpsc::UnboundedReceiver<(InitWithConfig, TxInitResponse<E>)>;
pub(crate) type RxChanPropose<E> = mpsc::UnboundedReceiver<(ProposeConfigChange, TxProposeResponse<E>)>;

/// The Raft API.
///
/// This type implements the full Raft spec, and is the interface into a running Raft node.
/// Applications building on top of Raft will use this to spawn a Raft task and interact with
/// the spawned task.
///
/// For more information on the Raft protocol, see the specification here:
/// https://raft.github.io/raft.pdf (**pdf warning**).
///
/// The beginning of §5, the spec has a condensed summary of the Raft consensus algorithm. This
/// crate, and especially this actor, attempts to follow the terminology and nomenclature used
/// there as precisely as possible to aid in understanding this system.
pub struct Raft<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> {
    tx_append_entries: TxChanAppendEntries<D, E>,
    tx_vote: TxChanVote<E>,
    tx_install_snapshot: TxChanInstallSnapshot<E>,
    tx_client: TxChanClient<D, R, E>,
    tx_init_with_config: TxChanInit<E>,
    tx_propose_config_change: TxChanPropose<E>,
    rx_metrics: watch::Receiver<RaftMetrics>,
    raft_handle: JoinHandle<RaftResult<(), E>>,
    _marker_n: std::marker::PhantomData<N>,
    _marker_s: std::marker::PhantomData<S>,
}

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> Raft<D, R, E, N, S> {
    /// Create and spawn a new Raft task.
    ///
    /// ### `id`
    /// The ID which the spawned Raft task will use to identify itself within the cluster.
    /// Applications must guarantee that the ID provided to this function is stable, and should be
    /// persisted in a well known location, probably alongside the Raft log and the application's
    /// state machine. This ensures that restarts of the node will yield the same ID every time.
    ///
    /// ### `config`
    /// The runtime config Raft. See the docs on the `Config` object for more details.
    ///
    /// ### `network`
    /// An implementation of the `RaftNetwork` trait which will be used by Raft for sending RPCs to
    /// peer nodes within the cluster. See the docs on the `RaftNetwork` trait for more details.
    ///
    /// ### `storage`
    /// An implementation of the `RaftStorage` trait which will be used by Raft for data storage.
    /// See the docs on the `RaftStorage` trait for more details.
    pub fn new(id: NodeId, config: Config, network: Arc<N>, storage: Arc<S>) -> Self {
        let (tx_append_entries, rx_append_entries) = mpsc::unbounded_channel();
        let (tx_vote, rx_vote) = mpsc::unbounded_channel();
        let (tx_install_snapshot, rx_install_snapshot) = mpsc::unbounded_channel();
        let (tx_client, rx_client) = mpsc::unbounded_channel();
        let (tx_init_with_config, rx_init_with_config) = mpsc::unbounded_channel();
        let (tx_propose_config_change, rx_propose_config_change) = mpsc::unbounded_channel();
        let (tx_metrics, rx_metrics) = watch::channel(RaftMetrics::new_initial(id));
        let raft_handle = RaftCore::spawn(
            id, config, network, storage,
            rx_append_entries, rx_vote, rx_install_snapshot, rx_client,
            rx_init_with_config, rx_propose_config_change, tx_metrics,
        );
        Self{
            tx_append_entries, tx_vote, tx_install_snapshot, tx_client,
            tx_init_with_config, tx_propose_config_change, rx_metrics, raft_handle,
            _marker_n: std::marker::PhantomData, _marker_s: std::marker::PhantomData,
        }
    }

    /// Submit an AppendEntries RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader to replicate log entries (§5.3), and are also
    /// used as heartbeats (§5.2).
    ///
    /// Applications are responsible for implementing a network layer which can receive the RPCs
    /// sent by Raft nodes via their `RaftNetwork` implementation. See the [networking section](TODO:)
    /// in the guide for more details.
    #[tracing::instrument(level="debug", skip(self, msg))]
    pub async fn append_entries(&self, msg: AppendEntriesRequest<D>) -> RaftResult<AppendEntriesResponse, E> {
        let (tx, rx) = oneshot::channel();
        self.tx_append_entries.send((msg, tx)).map_err(|_| RaftError::ShuttingDown)?;
        Ok(rx.await.map_err(|_| RaftError::ShuttingDown).and_then(|res| res)?)
    }

    /// Submit a VoteRequest (RequestVote in the spec) RPC to this Raft node.
    ///
    /// These RPCs are sent by cluster peers which are in candidate state attempting to gather votes (§5.2).
    ///
    /// Applications are responsible for implementing a network layer which can receive the RPCs
    /// sent by Raft nodes via their `RaftNetwork` implementation. See the [networking section](TODO:)
    /// in the guide for more details.
    #[tracing::instrument(level="debug", skip(self, msg))]
    pub async fn vote(&self, msg: VoteRequest) -> RaftResult<VoteResponse, E> {
        let (tx, rx) = oneshot::channel();
        self.tx_vote.send((msg, tx)).map_err(|_| RaftError::ShuttingDown)?;
        Ok(rx.await.map_err(|_| RaftError::ShuttingDown).and_then(|res| res)?)
    }

    /// Submit an InstallSnapshot RPC to this Raft node.
    ///
    /// These RPCs are sent by the cluster leader in order to bring a new node or a slow node up-to-speed
    /// with the leader (§7).
    ///
    /// Applications are responsible for implementing a network layer which can receive the RPCs
    /// sent by Raft nodes via their `RaftNetwork` implementation. See the [networking section](TODO:)
    /// in the guide for more details.
    #[tracing::instrument(level="debug", skip(self, msg))]
    pub async fn install_snapshot(&self, msg: InstallSnapshotRequest) -> RaftResult<InstallSnapshotResponse, E> {
        let (tx, rx) = oneshot::channel();
        self.tx_install_snapshot.send((msg, tx)).map_err(|_| RaftError::ShuttingDown)?;
        Ok(rx.await.map_err(|_| RaftError::ShuttingDown).and_then(|res| res)?)
    }

    /// Submit a client request to this Raft node to update the state of the system (§5.1).
    ///
    /// Client requests are application specific and should contain whatever type of data is needed
    /// by the application itself.
    ///
    /// Our goal for Raft is to implement linearizable semantics. If the leader crashes after committing
    /// a log entry but before responding to the client, the client may retry the command with a new
    /// leader, causing it to be executed a second time.
    ///
    /// The solution is for clients to assign unique serial numbers to every command. Then, the state
    /// machine tracks the latest serial number processed for each client, along with the associated
    /// response. If it receives a command whose serial number has already been executed, it responds
    /// immediately without reexecuting the request (§8).
    ///
    /// These are application specific requirements, and must be implemented by the application which is
    /// being built on top of Raft.
    #[tracing::instrument(level="debug", skip(self, msg))]
    pub async fn client(&self, msg: ClientRequest<D>) -> Result<ClientResponse<R>, ClientError<D, E>> {
        let (tx, rx) = oneshot::channel();
        self.tx_client.send((msg, tx)).map_err(|_| ClientError::RaftError(RaftError::ShuttingDown))?;
        Ok(rx.await.map_err(|_| ClientError::RaftError(RaftError::ShuttingDown)).and_then(|res| res)?)
    }

    /// Initialize a pristine Raft node with the given config.
    ///
    /// This command should be called on pristine nodes — where the log index is 0 and the node is
    /// in NonVoter state — as if either of those constraints are false, it indicates that the
    /// cluster is already formed and in motion. If `InitWithConfigError::NotAllowed` is returned
    /// from this function, it is safe to ignore, as it simply indicates that the cluster is
    /// already up and running, which is ultimately the goal of this function.
    ///
    /// This command will work for single-node or multi-node cluster formation. This command
    /// should be called with all discovered nodes which need to be part of cluster, and as such
    /// it is recommended that applications be configured with an initial cluster formation delay
    /// which will allow time for the initial members of the cluster to be discovered for this call.
    ///
    /// If successful, this routine will set the given config as the active config, only in memory,
    /// and will start an election.
    ///
    /// It is recommended that applications call this function based on an initial call to
    /// `RaftStorage.get_initial_state`. If the initial state indicates that the hard state's
    /// current term is `0` and the `last_log_index` is `0`, then this routine should be called
    /// in order to initialize the cluster.
    ///
    /// Once a node becomes leader and detects that its index is 0, it will commit a new config
    /// entry (instead of the normal blank entry created by new leaders).
    ///
    /// Every member of the cluster should perform these actions. This routine is race-condition
    /// free, and Raft guarantees that the first node to become the cluster leader will propage
    /// only its own config.
    ///
    /// Once a cluster is up and running, the `propose_config_change` routine should be used to
    /// update the cluster's membership config.
    #[tracing::instrument(level="debug", skip(self, msg))]
    pub async fn init_with_config(&self, msg: InitWithConfig) -> Result<(), InitWithConfigError<E>> {
        let (tx, rx) = oneshot::channel();
        self.tx_init_with_config.send((msg, tx)).map_err(|_| RaftError::ShuttingDown)?;
        Ok(rx.await.map_err(|_| InitWithConfigError::RaftError(RaftError::ShuttingDown)).and_then(|res| res)?)
    }

    /// Propose a cluster configuration change (§6).
    ///
    /// Applications built on top of Raft will typically have some peer discovery mechanism for
    /// detecting when new nodes are being added to the cluster, or when a node has been offline
    /// for some amount of time and should be removed from the cluster (cluster self-healing). Other
    /// applications may not have a dynamic membership system, but instead may have a manual method
    /// of updating a cluster's membership.
    ///
    /// An all of the above cases (as well as other cases not discussed), this method is used for
    /// proposing changes to the cluster's membership configuration.
    ///
    /// If this Raft node is not the cluster leader, then the proposed configuration change will be
    /// rejected.
    #[tracing::instrument(level="debug", skip(self, msg))]
    pub async fn propose_config_change(&self, msg: ProposeConfigChange) -> Result<(), ProposeConfigChangeError<E>> {
        let (tx, rx) = oneshot::channel();
        self.tx_propose_config_change.send((msg, tx)).map_err(|_| RaftError::ShuttingDown)?;
        Ok(rx.await.map_err(|_| ProposeConfigChangeError::RaftError(RaftError::ShuttingDown)).and_then(|res| res)?)
    }

    /// Get a handle to the metrics channel.
    pub fn metrics(&self) -> watch::Receiver<RaftMetrics> {
        self.rx_metrics.clone()
    }

    /// Get the Raft core JoinHandle, consuming this interface.
    pub fn core_handle(self) -> tokio::task::JoinHandle<RaftResult<(), E>> {
        self.raft_handle
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by a cluster leader to replicate log entries (§5.3), and as a heartbeat (§5.2).
#[derive(Debug, Serialize, Deserialize)]
pub struct AppendEntriesRequest<D: AppData> {
    /// The leader's current term.
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,
    /// The index of the log entry immediately preceding the new entries.
    pub prev_log_index: u64,
    /// The term of the `prev_log_index` entry.
    pub prev_log_term: u64,
    /// The new log entries to store.
    ///
    /// This may be empty when the leader is sending heartbeats. Entries
    /// are batched for efficiency.
    #[serde(bound="D: AppData")]
    pub entries: Vec<Entry<D>>,
    /// The leader's commit index.
    pub leader_commit: u64,
}

/// The response to an `AppendEntriesRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AppendEntriesResponse {
    /// The responding node's current term, for leader to update itself.
    pub term: u64,
    /// Will be true if follower contained entry matching `prev_log_index` and `prev_log_term`.
    pub success: bool,
    /// A value used to implement the _conflicting term_ optimization outlined in §5.3.
    ///
    /// This value will only be present, and should only be considered, when `success` is `false`.
    pub conflict_opt: Option<ConflictOpt>,
}

/// A struct used to implement the _conflicting term_ optimization outlined in §5.3 for log replication.
///
/// This value will only be present, and should only be considered, when an `AppendEntriesResponse`
/// object has a `success` value of `false`.
///
/// This implementation of Raft uses this value to more quickly synchronize a leader with its
/// followers which may be some distance behind in replication, may have conflicting entries, or
/// which may be new to the cluster.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConflictOpt {
    /// The term of the most recent entry which does not conflict with the received request.
    pub term: u64,
    /// The index of the most recent entry which does not conflict with the received request.
    pub index: u64,
}

/// A Raft log entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry<D: AppData> {
    /// This entry's term.
    pub term: u64,
    /// This entry's index.
    pub index: u64,
    /// This entry's payload.
    #[serde(bound="D: AppData")]
    pub payload: EntryPayload<D>,
}

impl<D: AppData> Entry<D> {
    /// Create a new snapshot pointer from the given data.
    pub fn new_snapshot_pointer(pointer: EntrySnapshotPointer, index: u64, term: u64) -> Self {
        Entry{term, index, payload: EntryPayload::SnapshotPointer(pointer)}
    }
}

/// Log entry payload variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EntryPayload<D: AppData> {
    /// An empty payload committed by a new cluster leader.
    Blank,
    /// A normal log entry.
    #[serde(bound="D: AppData")]
    Normal(EntryNormal<D>),
    /// A config change log entry.
    ConfigChange(EntryConfigChange),
    /// An entry which points to a snapshot.
    SnapshotPointer(EntrySnapshotPointer),
}

/// A normal log entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryNormal<D: AppData> {
    /// The contents of this entry.
    #[serde(bound="D: AppData")]
    pub data: D,
}

/// A log entry holding a config change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryConfigChange {
    /// Details on the cluster's membership configuration.
    pub membership: MembershipConfig,
}

/// A log entry pointing to a snapshot.
///
/// This will only be present when read from storage. An entry of this type will never be
/// transmitted from a leader during replication, an `InstallSnapshotRequest`
/// RPC will be sent instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntrySnapshotPointer {
    /// The ID of the snapshot, which is application specific, and probably only meaningful to the storage layer.
    pub id: String,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// A model of the membership configuration of the cluster.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipConfig {
    /// A flag indicating if the system is currently in a joint consensus state.
    pub is_in_joint_consensus: bool,
    /// Voting members of the Raft cluster.
    pub members: Vec<NodeId>,
    /// Non-voting members of the cluster.
    ///
    /// These nodes are being brought up-to-speed by the leader and will be transitioned over to
    /// being standard members once they are up-to-date.
    pub non_voters: Vec<NodeId>,
    /// The set of nodes which are to be removed after joint consensus is complete.
    pub removing: Vec<NodeId>,
}

impl MembershipConfig {
    /// Create a new initial config containing only the given node ID.
    pub(crate) fn new_initial(id: NodeId) -> Self {
        Self{is_in_joint_consensus: false, members: vec![id], non_voters: vec![], removing: vec![]}
    }

    /// Check if the given NodeId exists in this membership config.
    ///
    /// This checks only the contents of `members` & `non_voters`.
    pub fn contains(&self, x: &NodeId) -> bool {
        self.members.contains(x) || self.non_voters.contains(x)
    }

    /// Get an iterator over all nodes in the current config.
    pub fn all_nodes(&self) -> impl Iterator<Item=&NodeId> {
        self.members.iter().chain(self.non_voters.iter())
    }

    /// Get the length of the members & non_voters vectors.
    pub fn len(&self) -> usize {
        self.members.len() + self.non_voters.len()
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteRequest {
    /// The candidate's current term.
    pub term: u64,
    /// The candidate's ID.
    pub candidate_id: u64,
    /// The index of the candidate’s last log entry (§5.4).
    pub last_log_index: u64,
    /// The term of the candidate’s last log entry (§5.4).
    pub last_log_term: u64,
}

impl VoteRequest {
    /// Create a new instance.
    pub fn new(term: u64, candidate_id: u64, last_log_index: u64, last_log_term: u64) -> Self {
        Self{term, candidate_id, last_log_index, last_log_term}
    }
}

/// The response to a `VoteRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct VoteResponse {
    /// The current term of the responding node, for the candidate to update itself.
    pub term: u64,
    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,
    /// Will be true if the candidate is unknown to the responding node's config.
    ///
    /// If this field is true, and the sender's (the candidate's) index is greater than 0, then it
    /// should revert to the NonVoter state; if the sender's index is 0, then resume campaigning.
    pub is_candidate_unknown: bool,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// An RPC sent by the Raft leader to send chunks of a snapshot to a follower (§7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallSnapshotRequest {
    /// The leader's current term.
    pub term: u64,
    /// The leader's ID. Useful in redirecting clients.
    pub leader_id: u64,
    /// The snapshot replaces all log entries up through and including this index.
    pub last_included_index: u64,
    /// The term of the `last_included_index`.
    pub last_included_term: u64,
    /// The byte offset where this chunk of data is positioned in the snapshot file.
    pub offset: u64,
    /// The raw bytes of the snapshot chunk, starting at `offset`.
    pub data: Vec<u8>,
    /// Will be `true` if this is the last chunk in the snapshot.
    pub done: bool,
}

/// The response to an `InstallSnapshotRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallSnapshotResponse {
    /// The receiving node's current term, for leader to update itself.
    pub term: u64,
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// An application specific client request to update the state of the system (§5.1).
///
/// The entries of this payload will be appended to the Raft log and then applied to the Raft state
/// machine according to the Raft protocol.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientRequest<D: AppData> {
    /// The application specific contents of this client request.
    #[serde(bound="D: AppData")]
    pub(crate) entry: EntryPayload<D>,
    /// The response mode needed by this request.
    pub(crate) response_mode: ResponseMode,
}

impl<D: AppData> ClientRequest<D> {
    /// Create a new client payload instance with a normal entry type.
    pub fn new(entry: EntryNormal<D>, response_mode: ResponseMode) -> Self {
        Self::new_base(EntryPayload::Normal(entry), response_mode)
    }

    /// Create a new instance.
    pub(crate) fn new_base(entry: EntryPayload<D>, response_mode: ResponseMode) -> Self {
        Self{entry, response_mode}
    }

    /// Generate a new payload holding a config change.
    pub(crate) fn new_config(membership: MembershipConfig) -> Self {
        Self::new_base(EntryPayload::ConfigChange(EntryConfigChange{membership}), ResponseMode::Committed)
    }

    /// Generate a new blank payload.
    ///
    /// This is used by new leaders when first coming to power.
    pub(crate) fn new_blank_payload() -> Self {
        Self::new_base(EntryPayload::Blank, ResponseMode::Committed)
    }
}

/// The desired response mode for a client request.
///
/// Generally speaking, applications should just use `Applied`. It will allow for response data
/// to be returned to the client, and also ensures that reads will be immediately consistent.
///
/// This value specifies when a client request desires to receive its response from Raft. When
/// `Comitted` is chosen, the client request will receive a response after the request has been
/// successfully replicated to at least half of the nodes in the cluster. This is what the Raft
/// protocol refers to as being comitted.
///
/// When `Applied` is chosen, the client request will receive a response after the request has
/// been successfully committed and successfully applied to the state machine.
///
/// The choice between these two options depends on the requirements related to the request. If
/// a data response from the application's state machine needs to be returned, of if the data of
/// the client request payload will need to be read immediately after the response is
/// received, then `Applied` must be used. Otherwise `Committed` may be used to speed up
/// response times. All things considered, the difference in response time will just be the amount
/// of times it takes to apply the payload to the state machine, and may not be significant.
#[derive(Debug, Serialize, Deserialize)]
pub enum ResponseMode {
    /// A response will be returned after the request has been committed to the cluster.
    Committed,
    /// A response will be returned after the request has been applied to the leader's state machine.
    Applied,
}

/// The response to a `ClientRequest`.
#[derive(Debug, Serialize, Deserialize)]
pub enum ClientResponse<R: AppDataResponse> {
    /// A client response issued just after the request was committed to the cluster.
    Committed {
        /// The log index of the successfully processed client request.
        index: u64,
    },
    Applied {
        /// The log index of the successfully processed client request.
        index: u64,
        /// Application specific response data.
        #[serde(bound="R: AppDataResponse")]
        data: R,
    },
}

impl<R: AppDataResponse> ClientResponse<R> {
    /// The index of the log entry corresponding to this response object.
    pub fn index(&self) -> u64 {
        match self {
            Self::Committed{index} => *index,
            Self::Applied{index, ..} => *index,
        }
    }

    /// The response data payload, if this is an `Applied` client response.
    pub fn data(&self) -> Option<&R> {
        match &self {
            Self::Committed{..} => None,
            Self::Applied{data, ..} => Some(data),
        }
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// The data model used for initializing a pristine Raft node.
pub struct InitWithConfig {
    /// All currently known members to initialize the new cluster with.
    ///
    /// If the ID of the node this command is being submitted to is not present, it will be added.
    /// If there are duplicates, they will be filtered out to ensure config is proper.
    pub(crate) members: Vec<NodeId>,
}

impl InitWithConfig {
    /// Construct a new instance.
    pub fn new(members: Vec<NodeId>) -> Self {
        Self{members}
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
//////////////////////////////////////////////////////////////////////////////////////////////////

/// Propose a new membership config change to a running cluster.
///
/// There are a few invariants which must be upheld here:
///
/// - if the node this command is sent to is not the leader of the cluster, it will be rejected.
/// - if the given changes would leave the cluster in an inoperable state, it will be rejected.
pub struct ProposeConfigChange {
    /// New members to be added to the cluster.
    pub(crate) add_members: Vec<NodeId>,
    /// Members to be removed from the cluster.
    pub(crate) remove_members: Vec<NodeId>,
}

impl ProposeConfigChange {
    /// Create a new instance.
    ///
    /// If there are duplicates in either of the givenn vectors, they will be filtered out to
    /// ensure config is proper.
    pub fn new(add_members: Vec<NodeId>, remove_members: Vec<NodeId>) -> Self {
        Self{add_members, remove_members}
    }
}
