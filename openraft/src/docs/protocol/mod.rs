//! The protocol used by Openraft to replicate data.

pub mod replication {
    #![doc = include_str!("replication.md")]

    pub mod log_replication {
        #![doc = include_str!("log_replication.md")]
    }

    pub mod snapshot_replication {
        #![doc = include_str!("snapshot_replication.md")]
    }
}

pub mod fast_commit {
    #![doc = include_str!("fast_commit.md")]
}
