//! Bind openraft's types to the type slots `raft_log` declares.

use std::marker::PhantomData;

use openraft::RaftTypeConfig;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;

use crate::Callback;
use crate::codec::MsgPack;
use crate::codec::MsgPackVote;

/// Fills in `raft_log::Types` with the log id, entry and vote types of the
/// openraft type config `C`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WalTypes<C>(PhantomData<C>);

impl<C> raft_log::Types for WalTypes<C>
where
    C: RaftTypeConfig,
    EntryOf<C>: Clone,
{
    type LogId = MsgPack<LogIdOf<C>>;

    /// The whole entry is the payload.
    ///
    /// An entry already carries its own log id, so the id is stored twice: once
    /// in the `LogId` slot that `raft_log` indexes by, and once inside the
    /// payload. A store that owns its entry format would split the id off and
    /// keep only the command here.
    type LogPayload = MsgPack<EntryOf<C>>;

    type Vote = MsgPackVote<C>;
    type Callback = Callback<C>;

    /// This store attaches no application data to the log.
    type UserData = MsgPack<()>;

    fn log_index(log_id: &Self::LogId) -> u64 {
        log_id.0.index
    }

    fn payload_size(payload: &Self::LogPayload) -> u64 {
        // `raft_log` evicts from its payload cache against this number. A fixed
        // per-entry estimate would ignore everything an entry holds on the heap
        // and let the cache grow past its configured capacity, so the entry is
        // measured instead. The cost is one MessagePack pass per cache insert
        // and per eviction. A store that keeps the encoded bytes around reads
        // their length off the buffer and pays nothing.
        payload.encoded_len()
    }
}
