use crate::engine::LogIdList;
use crate::leader::Leader;
use crate::raft_types::RaftLogId;
use crate::LogId;
use crate::MembershipState;
use crate::NodeId;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::ServerState;
use crate::StorageError;
use crate::Vote;

/// A struct used to represent the raft state which a Raft node needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaftState<NID: NodeId> {
    /// The vote state of this node.
    pub vote: Vote<NID>,

    /// The greatest log id that has been purged after being applied to state machine.
    /// The range of log entries that exist in storage is `(last_purged_log_id, last_log_id]`,
    /// left open and right close.
    ///
    /// `last_purged_log_id == last_log_id` means there is no log entry in the storage.
    pub last_purged_log_id: Option<LogId<NID>>,

    /// The id of the last log entry.
    pub last_log_id: Option<LogId<NID>>,

    /// The LogId of the last log applied to the state machine.
    pub last_applied: Option<LogId<NID>>,

    /// All log ids this node has.
    pub log_ids: LogIdList<NID>,

    /// The latest cluster membership configuration found, in log or in state machine.
    pub membership_state: MembershipState<NID>,

    // --
    // -- volatile fields: they are not persisted.
    // --
    /// The state a leader would have.
    ///
    /// In openraft there are only two state for a server:
    /// Leader and follower:
    ///
    /// - A leader is able to vote(candidate in original raft) and is able to propose new log if its vote is granted by
    ///   quorum(leader in original raft).
    ///
    ///   In this way the leadership won't be lost when it sees a higher `vote` and needs upgrade its `vote`.
    ///
    /// - A follower will just receive replication from a leader. A follower that is one of the member will be able to
    ///   become leader. A follower that is not a member is just a learner.
    pub leader: Option<Leader<NID>>,

    /// The log id of the last known committed entry.
    ///
    /// - Committed means: a log that is replicated to a quorum of the cluster and it is of the term of the leader.
    ///
    /// - A quorum could be a uniform quorum or joint quorum.
    ///
    /// - `committed` in raft is volatile and will not be persisted.
    pub committed: Option<LogId<NID>>,

    pub server_state: ServerState,
}

impl<NID: NodeId> RaftState<NID> {
    /// Load all log ids that are the first one proposed by a leader.
    ///
    /// E.g., log ids with the same leader id will be got rid of, except the smallest.
    /// The `last_log_id` will always present at the end, to simplify searching.
    ///
    /// Given an example with the logs `[(2,2),(2,3),(5,4),(5,5)]`, and the `last_purged_log_id` is (1,1).
    /// This function returns `[(1,1),(2,2),(5,4),(5,5)]`.
    ///
    /// It adopts a modified binary-search algo.
    /// ```text
    /// input:
    /// A---------------C
    ///
    /// load the mid log-id, then compare the first, the middle, and the last:
    ///
    /// A---------------A : push_res(A);
    /// A-------A-------C : push_res(A); find(A,C) // both find `A`, need to de-dup
    /// A-------B-------C : find(A,B); find(B,C)   // both find `B`, need to de-dup
    /// A-------C-------C : find(A,C)
    /// ```
    pub async fn load_log_ids<C, Sto>(
        last_purged_log_id: Option<LogId<NID>>,
        last_log_id: Option<LogId<NID>>,
        sto: &mut Sto,
    ) -> Result<LogIdList<NID>, StorageError<NID>>
    where
        C: RaftTypeConfig<NodeId = NID>,
        Sto: RaftStorage<C> + ?Sized,
    {
        let mut res = vec![];

        let last = match last_log_id {
            None => return Ok(LogIdList::new(res)),
            Some(x) => x,
        };
        let first = match last_purged_log_id {
            None => sto.get_log_id(0).await?,
            Some(x) => x,
        };

        // Recursion stack
        let mut stack = vec![(first, last)];

        loop {
            let (first, last) = match stack.pop() {
                None => {
                    break;
                }
                Some(x) => x,
            };

            // Case AA
            if first.leader_id == last.leader_id {
                if res.last().map(|x| x.leader_id) < Some(first.leader_id) {
                    res.push(first);
                }
                continue;
            }

            // Two adjacent logs with different leader_id, no need to binary search
            if first.index + 1 == last.index {
                if res.last().map(|x| x.leader_id) < Some(first.leader_id) {
                    res.push(first);
                }
                res.push(last);
                continue;
            }

            let mid = sto.get_log_id((first.index + last.index) / 2).await?;

            if first.leader_id == mid.leader_id {
                // Case AAC
                if res.last().map(|x| x.leader_id) < Some(first.leader_id) {
                    res.push(first);
                }
                stack.push((mid, last));
            } else if mid.leader_id == last.leader_id {
                // Case ACC
                stack.push((first, mid));
            } else {
                // Case ABC
                // first.leader_id < mid_log_id.leader_id < last.leader_id
                // Deal with (first, mid) then (mid, last)
                stack.push((mid, last));
                stack.push((first, mid));
            }
        }

        if res.last() != Some(&last) {
            res.push(last);
        }

        Ok(LogIdList::new(res))
    }

    /// Append a list of `log_id`.
    ///
    /// The log ids in the input has to be continuous.
    pub(crate) fn extend_log_ids_from_same_leader<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_log_id: &[LID]) {
        self.log_ids.extend_from_same_leader(new_log_id)
    }

    #[allow(dead_code)]
    pub(crate) fn extend_log_ids<'a, LID: RaftLogId<NID> + 'a>(&mut self, new_log_id: &[LID]) {
        self.log_ids.extend(new_log_id)
    }

    /// Get the log id at the specified index.
    ///
    /// It will return `last_purged_log_id` if index is at the last purged index.
    #[allow(dead_code)]
    pub(crate) fn get_log_id(&self, index: u64) -> Option<LogId<NID>> {
        self.log_ids.get(index)
    }
}
