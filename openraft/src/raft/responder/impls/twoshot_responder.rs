use crate::LogId;
use crate::OptionalSend;
use crate::RaftTypeConfig;
use crate::async_runtime::OneshotSender;
use crate::raft::responder::Responder;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::OneshotReceiverOf;
use crate::type_config::alias::OneshotSenderOf;

/// A [`Responder`] implementation that sends notifications via two oneshot channels.
///
/// This responder provides both commit and completion notifications:
/// - **Commit channel**: Notifies when the log entry is committed (replicated to a quorum)
/// - **Complete channel**: Sends the final result when the request completes
///
/// Use this when the caller wants to be notified at both stages:
/// 1. When the entry is committed and safe to read
/// 2. When the entry is applied and the result is available
///
/// # Example
///
/// ```ignore
/// let (responder, commit_rx, complete_rx) = ProgressResponder::new();
///
/// // Send write request with the responder
/// raft.client_write_ff(request, responder).await;
///
/// // Wait for commit notification
/// let commit_log_id = commit_rx.await.unwrap();
/// // Now safe to read the committed data
///
/// // Wait for completion
/// let result = complete_rx.await.unwrap();
/// // Now have the final result
/// ```
pub struct ProgressResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend,
{
    commit_tx: Option<OneshotSenderOf<C, LogId<C>>>,
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
    pub fn new() -> (Self, OneshotReceiverOf<C, LogId<C>>, OneshotReceiverOf<C, T>) {
        let (commit_tx, commit_rx) = C::oneshot();
        let (complete_tx, complete_rx) = C::oneshot();

        let responder = Self {
            commit_tx: Some(commit_tx),
            complete_tx,
        };

        (responder, commit_rx, complete_rx)
    }
}

impl<C, T> Responder<C, T> for ProgressResponder<C, T>
where
    C: RaftTypeConfig,
    T: OptionalSend + 'static,
{
    fn on_commit(&mut self, log_id: LogId<C>) {
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
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::raft::responder::ProgressResponder;
    use crate::raft::responder::Responder;

    #[tokio::test]
    async fn test_twoshot_responder_new() {
        let (_responder, mut commit_rx, mut complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
            ProgressResponder::new();

        // Receivers should be created but not yet have values
        assert!(commit_rx.try_recv().is_err());
        assert!(complete_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_twoshot_responder_on_commit() {
        let (mut responder, commit_rx, _complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
            ProgressResponder::new();

        let test_log_id = log_id(1, 2, 3);

        // Send commit notification
        responder.on_commit(test_log_id);

        // Commit receiver should receive the log_id
        let received_log_id = commit_rx.await.unwrap();
        assert_eq!(test_log_id, received_log_id);
    }

    #[tokio::test]
    async fn test_twoshot_responder_on_commit_multiple_calls() {
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
    }

    #[tokio::test]
    async fn test_twoshot_responder_send() {
        let (responder, _commit_rx, complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
            ProgressResponder::new();

        let test_result = "test_result".to_string();

        // Send completion result
        responder.on_complete(test_result.clone());

        // Complete receiver should receive the result
        let received_result = complete_rx.await.unwrap();
        assert_eq!(test_result, received_result);
    }

    #[tokio::test]
    async fn test_twoshot_responder_both_channels() {
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
    }

    #[tokio::test]
    async fn test_twoshot_responder_send_without_commit() {
        let (responder, mut commit_rx, complete_rx): (ProgressResponder<UTConfig, String>, _, _) =
            ProgressResponder::new();

        let test_result = "test_result".to_string();

        // Send completion without calling on_commit
        responder.on_complete(test_result.clone());

        // Complete receiver should still receive the result
        let received_result = complete_rx.await.unwrap();
        assert_eq!(test_result, received_result);

        // Commit receiver should not have received anything
        assert!(commit_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_twoshot_responder_ordering() {
        let (mut responder, commit_rx, complete_rx): (ProgressResponder<UTConfig, i32>, _, _) =
            ProgressResponder::new();

        let test_log_id = log_id(5, 10, 15);
        let test_result = 42;

        // Create tasks to receive in parallel
        let commit_task = tokio::spawn(async move { commit_rx.await.unwrap() });

        let complete_task = tokio::spawn(async move { complete_rx.await.unwrap() });

        // Small delay to ensure receivers are waiting
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Send in order: commit first, then complete
        responder.on_commit(test_log_id);
        responder.on_complete(test_result);

        // Both should complete successfully
        let received_log_id = commit_task.await.unwrap();
        let received_result = complete_task.await.unwrap();

        assert_eq!(test_log_id, received_log_id);
        assert_eq!(test_result, received_result);
    }
}
