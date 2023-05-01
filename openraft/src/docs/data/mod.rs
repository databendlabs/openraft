//! Data structures used by the Openraft protocol, such log, vote, snapshot, membership etc.

pub mod leader_id {
    #![doc = include_str!("leader_id.md")]
}

pub mod vote {
    #![doc = include_str!("vote.md")]
}

pub mod log_pointers {
    #![doc = include_str!("log_pointers.md")]
}

pub mod extended_membership {
    #![doc = include_str!("extended-membership.md")]
}

pub mod effective_membership {
    #![doc = include_str!("effective-membership.md")]
}
