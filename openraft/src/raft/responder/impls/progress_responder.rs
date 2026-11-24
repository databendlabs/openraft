use openraft_macros::since;

use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::OneshotSender;
use crate::raft::responder::Responder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;

/// A [`Responder`] implementation that sends notifications via oneshot channels.
///
/// This responder can provide both commit and completion notifications:
/// - **Commit channel**: Notifies when the log entry is committed (replicated to a quorum)
/// - **Complete channel**: Sends the final result when the request completes
///
/// Use [`ProgressResponder::new()`] when the caller wants to be notified at both stages:
/// 1. When the entry is committed, but not necessarily applied
/// 2. When the entry is applied and the result is available
///
/// The commit notification is optional. A write rejected before it is committed, such as one
/// submitted to a follower, completes with its error and no commit notification. Completion drops
/// the unused commit sender, so the commit receiver then returns an error instead of a log id.
///
/// If the responder is dropped without a result, such as when RaftCore stops, the complete receiver
/// returns an error. No result arrived, but that alone does not mean that the write was rolled
/// back: its entry may still be committed and applied.
///
/// With relaxed reads
/// ([`LinearizerOption::with_relaxed`](crate::raft::linearizable_read::LinearizerOption::with_relaxed)),
/// commit notification is not enough to acknowledge a successful write. State-machine reads after
/// the write response must include the entry's updates. The store must publish them before
/// completion, or the application must delay its client-visible acknowledgement.
///
/// Use [`ProgressResponder::complete_only()`] when the caller only needs the final result.
///
/// # Example
///
/// [`Raft::client_write_ff`] accepts the responder type that the type config selects as
/// `Responder<T>`. [`declare_raft_types!`] selects `ProgressResponder` by default.
///
/// ```no_run
/// # use openraft::Raft;
/// # use openraft::storage::RaftStateMachine;
/// use openraft::impls::ProgressResponder;
///
/// # openraft::declare_raft_types!(
/// #     TypeConfig:
/// #         Responder<T> = ProgressResponder<Self, T>,
/// # );
/// #
/// # async fn example<SM>(
/// #     raft: &Raft<TypeConfig, SM>,
/// #     request: String,
/// # ) -> Result<(), Box<dyn std::error::Error>>
/// # where SM: RaftStateMachine<TypeConfig>
/// # {
/// let (responder, commit_rx, complete_rx) = ProgressResponder::new();
///
/// // `Err` means that RaftCore has stopped and did not receive the write.
/// raft.client_write_ff(request, Some(responder)).await?;
///
/// // A write that completes without being committed, such as one submitted to a follower, closes
/// // the commit channel without a log id.
/// if let Ok(log_id) = commit_rx.await {
///     // Committed, but maybe not applied: do not acknowledge the write from this alone.
///     println!("committed at {log_id}");
/// }
///
/// let result = match complete_rx.await {
///     Ok(result) => result,
///     // No result arrived. The write may still be committed and applied.
///     Err(_) => return Err("no result arrived for the write".into()),
/// };
///
/// // The result arrived: the applied write's response, or the write's error, such as
/// // `ForwardToLeader` from a follower.
/// let response = result?;
/// println!("applied at {}: {}", response.log_id, response.data);
/// # Ok(())
/// # }
/// ```
///
/// [`Raft::client_write_ff`]: crate::Raft::client_write_ff
/// [`declare_raft_types!`]: crate::declare_raft_types
pub struct ProgressResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend,
{
    commit_tx: Option<OneshotSenderOf<C, LogIdOf<C>>>,
    complete_tx: OneshotSenderOf<C, T>,
}

impl<C, T> ProgressResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend,
{
    /// Create a new responder with commit and complete receivers.
    ///
    /// This is a convenience method that creates two oneshot channels and returns
    /// a [`ProgressResponder`] wrapping both senders, along with both receivers.
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// - The [`ProgressResponder`] that can send both commit and complete notifications
    /// - The commit receiver for receiving the committed log ID
    /// - The complete receiver for receiving the final result
    pub fn new() -> (Self, OneshotReceiverOf<C, LogIdOf<C>>, OneshotReceiverOf<C, T>) {
        let (commit_tx, commit_rx) = C::oneshot();
        let (complete_tx, complete_rx) = C::oneshot();

        let responder = Self {
            commit_tx: Some(commit_tx),
            complete_tx,
        };

        (responder, commit_rx, complete_rx)
    }

    /// Create a new responder with only a complete receiver.
    ///
    /// Commit notifications are ignored. Use this when callers only wait for
    /// the final result and intentionally do not need commit progress.
    #[since(version = "0.10.0", change = "added complete_only constructor")]
    pub fn complete_only() -> (Self, OneshotReceiverOf<C, T>) {
        let (complete_tx, complete_rx) = C::oneshot();

        let responder = Self {
            commit_tx: None,
            complete_tx,
        };

        (responder, complete_rx)
    }
}

impl<C, T> Responder<C, T> for ProgressResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend + 'static,
{
    fn on_commit(&mut self, log_id: LogIdOf<C>) {
        if let Some(tx) = self.commit_tx.take() {
            let res = tx.send(log_id);

            if res.is_ok() {
                tracing::debug!("ProgressResponder.commit_tx.send: is_ok: {}", res.is_ok());
            } else {
                tracing::warn!("ProgressResponder.commit_tx.send: is_ok: {}", res.is_ok());
            }
        }
    }

    fn on_complete(self, res: T) {
        let res = self.complete_tx.send(res);

        if res.is_ok() {
            tracing::debug!("ProgressResponder.complete_tx.send: is_ok: {}", res.is_ok());
        } else {
            tracing::warn!("ProgressResponder.complete_tx.send: is_ok: {}", res.is_ok());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::oneshot::error::TryRecvError;

    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::raft::responder::ProgressResponder;
    use crate::raft::responder::Responder;
    use crate::type_config::TypeConfigExt;

    #[test]
    fn test_twoshot_responder_new() {
        UTConfig::<()>::run(async {
            let (_responder, mut commit_rx, mut complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
                ProgressResponder::new();

            // The live responder holds both senders, so both channels are open and still empty.
            let commit = commit_rx.try_recv();
            assert_eq!(Err(TryRecvError::Empty), commit);

            let complete = complete_rx.try_recv();
            assert_eq!(Err(TryRecvError::Empty), complete);
        });
    }

    #[test]
    fn test_twoshot_responder_complete_only_new() {
        UTConfig::<()>::run(async {
            let (_responder, mut complete_rx): (ProgressResponder<UTConfig, String>, _) =
                ProgressResponder::complete_only();

            // The live responder holds the sender, so the channel is open and still empty.
            let complete = complete_rx.try_recv();
            assert_eq!(Err(TryRecvError::Empty), complete);
        });
    }

    #[test]
    fn test_twoshot_responder_complete_only_ignores_commit() {
        UTConfig::<()>::run(async {
            let (mut responder, complete_rx): (ProgressResponder<UTConfig, String>, _) =
                ProgressResponder::complete_only();

            let test_log_id = log_id(1, 2, 3);
            let test_result = "test_result".to_string();

            // The complete-only responder intentionally has no commit receiver.
            responder.on_commit(test_log_id);
            responder.on_complete(test_result.clone());

            let received_result = complete_rx.await.unwrap();
            assert_eq!(test_result, received_result);
        });
    }

    #[test]
    fn test_twoshot_responder_on_commit() {
        UTConfig::<()>::run(async {
            let (mut responder, commit_rx, _complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
                ProgressResponder::new();

            let test_log_id = log_id(1, 2, 3);

            // Send commit notification
            responder.on_commit(test_log_id);

            // Commit receiver should receive the log_id
            let received_log_id = commit_rx.await.unwrap();
            assert_eq!(test_log_id, received_log_id);
        });
    }

    #[test]
    fn test_twoshot_responder_on_commit_multiple_calls() {
        UTConfig::<()>::run(async {
            let (mut responder, commit_rx, _complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
                ProgressResponder::new();

            let test_log_id_1 = log_id(1, 2, 3);
            let test_log_id_2 = log_id(2, 3, 4);

            // Send first commit notification
            responder.on_commit(test_log_id_1);

            // Second call should be ignored (tx is taken on first call)
            responder.on_commit(test_log_id_2);

            // Commit receiver should only receive the first log_id
            let received_log_id = commit_rx.await.unwrap();
            assert_eq!(test_log_id_1, received_log_id);
        });
    }

    #[test]
    fn test_twoshot_responder_send() {
        UTConfig::<()>::run(async {
            let (responder, _commit_rx, complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
                ProgressResponder::new();

            let test_result = "test_result".to_string();

            // Send completion result
            responder.on_complete(test_result.clone());

            // Complete receiver should receive the result
            let received_result = complete_rx.await.unwrap();
            assert_eq!(test_result, received_result);
        });
    }

    #[test]
    fn test_twoshot_responder_both_channels() {
        UTConfig::<()>::run(async {
            let (mut responder, commit_rx, complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
                ProgressResponder::new();

            let test_log_id = log_id(1, 2, 3);
            let test_result = "test_result".to_string();

            // Send commit notification
            responder.on_commit(test_log_id);

            // Verify commit was received
            let received_log_id = commit_rx.await.unwrap();
            assert_eq!(test_log_id, received_log_id);

            // Send completion result
            responder.on_complete(test_result.clone());

            // Verify completion was received
            let received_result = complete_rx.await.unwrap();
            assert_eq!(test_result, received_result);
        });
    }

    #[test]
    fn test_twoshot_responder_send_without_commit() {
        UTConfig::<()>::run(async {
            let (responder, mut commit_rx, complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
                ProgressResponder::new();

            let test_result = "test_result".to_string();

            // Send completion without calling on_commit
            responder.on_complete(test_result.clone());

            // Complete receiver should still receive the result
            let received_result = complete_rx.await.unwrap();
            assert_eq!(test_result, received_result);

            // Completion consumed the responder and dropped the unused commit sender, so the
            // commit channel is closed without a value.
            let commit = commit_rx.try_recv();
            assert_eq!(Err(TryRecvError::Closed), commit);
        });
    }

    #[test]
    fn test_twoshot_responder_ordering() {
        UTConfig::<()>::run(async {
            let (mut responder, commit_rx, complete_rx): (ProgressResponder<UTConfig, i32>, _, _) =
                ProgressResponder::new();

            let test_log_id = log_id(5, 10, 15);
            let test_result = 42;

            // Create tasks to receive in parallel
            let commit_task = UTConfig::<()>::spawn(async move { commit_rx.await.unwrap() });

            let complete_task = UTConfig::<()>::spawn(async move { complete_rx.await.unwrap() });

            // Small delay to ensure receivers are waiting
            UTConfig::<()>::sleep(Duration::from_millis(10)).await;

            // Send in order: commit first, then complete
            responder.on_commit(test_log_id);
            responder.on_complete(test_result);

            // Both should complete successfully
            let received_log_id = commit_task.await.unwrap();
            let received_result = complete_task.await.unwrap();

            assert_eq!(test_log_id, received_log_id);
            assert_eq!(test_result, received_result);
        });
    }
}
