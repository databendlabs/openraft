mod leader_id;
#[allow(clippy::module_inception)]
mod vote;

pub use leader_id::LeaderId;
pub use vote::Vote;

#[cfg(test)]
mod leader_id_test;
