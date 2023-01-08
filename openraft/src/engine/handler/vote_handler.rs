use crate::engine::engine_impl::EngineOutput;
use crate::engine::Command;
use crate::Node;
use crate::NodeId;
use crate::RaftState;

/// Handle raft vote related operations
pub(crate) struct VoteHandler<'st, 'out, NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) state: &'st mut RaftState<NID, N>,
    pub(crate) output: &'out mut EngineOutput<NID, N>,
}

impl<'st, 'out, NID, N> VoteHandler<'st, 'out, NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Mark the vote as committed, i.e., being granted and saved by a quorum.
    ///
    /// The committed vote, is not necessary in original raft.
    /// Openraft insists doing this because:
    /// - Voting is not in the hot path, thus no performance penalty.
    /// - Leadership won't be lost if a leader restarted quick enough.
    pub(crate) fn commit(&mut self) {
        debug_assert!(!self.state.vote.committed);

        self.state.vote.commit();
        self.output.push_command(Command::SaveVote { vote: self.state.vote });
    }
}
