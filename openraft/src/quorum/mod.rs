mod quorum_set;
mod quorum_set_impl;
mod util;

#[cfg(test)] mod quorum_set_test;
#[cfg(test)] mod util_test;

pub(crate) use quorum_set::QuorumSet;
pub(crate) use util::majority_of;
