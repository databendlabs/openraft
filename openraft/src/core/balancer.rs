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
        // Grow even when integer division rounds the proportional increase down to zero.
        let increase = (self.raft_msg / 16).max(1);
        self.raft_msg = (self.raft_msg + increase).min(self.total / 2);
    }
}

#[cfg(test)]
mod tests {
    use super::Balancer;

    #[test]
    fn test_raft_msg_budget_recovers_from_one() {
        let total = 10_000;
        let ceiling = total / 2;
        let mut balancer = Balancer::new(total);

        tracing::info!(total, "drive the API budget down with sustained notification demand");
        {
            for _ in 0..128 {
                balancer.increase_notification();
            }
            assert_eq!(balancer.raft_msg(), 1);
        }

        tracing::info!(
            ceiling,
            "reverse demand and grow the API budget from one to its ceiling"
        );
        {
            for expected in 2..=16 {
                balancer.increase_raft_msg();
                assert_eq!(balancer.raft_msg(), expected);
            }

            while balancer.raft_msg() < ceiling {
                let previous = balancer.raft_msg();
                balancer.increase_raft_msg();
                assert_eq!(balancer.raft_msg(), (previous * 17 / 16).min(ceiling));
            }
        }

        tracing::info!(ceiling, "preserve the notification share under continued API demand");
        {
            for _ in 0..16 {
                balancer.increase_raft_msg();
                assert_eq!(balancer.raft_msg(), ceiling);
                assert_eq!(balancer.notification(), total - ceiling);
            }
        }
    }
}
