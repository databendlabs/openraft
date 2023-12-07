//! The protocol used by Openraft to replicate data.

pub mod read {
    #![doc = include_str!("read.md")]
}

pub mod replication {
    #![doc = include_str!("replication.md")]

    pub mod leader_lease {
        #![doc = include_str!("leader_lease.md")]
    }

    pub mod log_replication {
        #![doc = include_str!("log_replication.md")]
    }

    pub mod snapshot_replication {
        #![doc = include_str!("snapshot_replication.md")]
    }
}
