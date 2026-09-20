//! The future type of a spawned task.

use std::future::Future;

use openraft_rt::OptionalSend;

/// A spawned task, as the executor stores it.
pub(crate) trait TaskFuture: Future<Output = ()> + OptionalSend {}

impl<F> TaskFuture for F where F: Future<Output = ()> + OptionalSend {}
