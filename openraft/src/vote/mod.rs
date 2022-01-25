#[allow(clippy::module_inception)]
mod vote;

pub use vote::CommittedState;
pub use vote::Vote;

#[cfg(test)]
mod vote_test;
