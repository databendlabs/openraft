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

    fn payload_size(_payload: &Self::LogPayload) -> u64 {
        // `raft_log` uses this number only to bound how much memory the payload
        // cache holds, and its doc says the number may be inaccurate. Measuring
        // an entry would mean encoding it on every call, so a fixed per-entry
        // estimate is used instead. A store that keeps the encoded bytes around
        // should report their real length.
        size_of::<EntryOf<C>>() as u64
    }
}
