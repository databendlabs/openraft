//! This mod defines and manipulates the ratio of multiple messages to handle.
//!
//! The ratio should be adjust so that every channel won't be starved.

/// Balance the ratio of different kind of message to handle.
pub(crate) struct Balancer {
    total: u64,

    /// The number of RaftMsg to handle in each round.
    raft_msg: u64,
}

impl Balancer {
    pub(crate) fn new(total: u64) -> Self {
        Self {
            total,
            // RaftMsg is the input entry.
            // We should consume as many as internal messages as possible.
            raft_msg: total / 10,
        }
    }

    pub(crate) fn raft_msg(&self) -> u64 {
        self.raft_msg
    }

    pub(crate) fn notification(&self) -> u64 {
        self.total - self.raft_msg
    }

    pub(crate) fn increase_notification(&mut self) {
        self.raft_msg = self.raft_msg * 15 / 16;
        if self.raft_msg == 0 {
            self.raft_msg = 1;
        }
    }

    pub(crate) fn increase_raft_msg(&mut self) {
        self.raft_msg = self.raft_msg * 17 / 16;

        // Always leave some budget for other channels
        if self.raft_msg > self.total / 2 {
            self.raft_msg = self.total / 2;
        }
    }
}
