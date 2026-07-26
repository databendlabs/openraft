//! Bringing a Raft node up and down, and reaching into its core state.

use std::fmt::Debug;

use openraft_macros::since;

use crate::OptionalSend;
use crate::Raft;
use crate::RaftState;
use crate::RaftTypeConfig;
use crate::async_runtime::MpscWeakSender;
use crate::async_runtime::OneshotSender;
use crate::async_runtime::mpsc::MpscSender;
use crate::base::BoxFuture;
use crate::base::BoxOnce;
use crate::core::raft_msg::RaftMsg;
use crate::core::sm;
use crate::errors::Fatal;
use crate::errors::InitializeError;
use crate::errors::RaftError;
use crate::errors::into_raft_result::IntoRaftResult;
use crate::membership::IntoNodes;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::JoinErrorOf;

/// Implement cluster initialization and shutdown, plus the escape hatches that run a closure
/// against `RaftCore`'s state or the state machine.
impl<C, SM> Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Return `true` if this node is already initialized and cannot be initialized again with
    /// [`Raft::initialize`]
    #[since(version = "0.10.0")]
    pub async fn is_initialized(&self) -> Result<bool, Fatal<C>> {
        let initialized = self.with_raft_state(|st| st.is_initialized()).await?;

        Ok(initialized)
    }

    /// Initialize a pristine Raft node with the given config.
    ///
    /// This command should be called on pristine nodes — where the log index is 0 and the node is
    /// in Learner state — as if either of those constraints are false, it indicates that the
    /// cluster is already formed and in motion. If `InitializeError::NotAllowed` is returned
    /// from this function, it is safe to ignore, as it simply indicates that the cluster is
    /// already up and running, which is ultimately the goal of this function. You can check
    /// if the cluster is initialized with [`Raft::is_initialized()`] and then avoid re-initialize
    /// it in case you want to get rid of this error.
    ///
    /// ## Recommended Usage
    ///
    /// The simplest and most appropriate way to initialize a cluster is to call `initialize()`
    /// on **exactly one node**. The other nodes should remain empty and wait for the initialized
    /// node to replicate logs to them.
    ///
    /// Calling `initialize()` on multiple nodes with **identical configuration** is also
    /// acceptable and will not cause any consistency issues — the Raft voting protocol ensures
    /// that only one leader will be elected.
    ///
    /// However, calling `initialize()` with **different configurations** on different nodes
    /// may lead to a split-brain condition and must be avoided.
    ///
    /// ## Behavior
    ///
    /// Once a node is successfully initialized, it will commit a new membership config
    /// log entry to store, then enter Candidate state and attempt to elect itself as the
    /// leader.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::collections::BTreeMap;
    /// use openraft::BasicNode;
    ///
    /// // Initialize a single-node cluster
    /// let mut nodes = BTreeMap::new();
    /// nodes.insert(1, BasicNode { addr: "127.0.0.1:8080".to_string() });
    /// raft.initialize(nodes).await?;
    ///
    /// // Initialize a three-node cluster
    /// let mut nodes = BTreeMap::new();
    /// nodes.insert(1, BasicNode { addr: "127.0.0.1:8080".to_string() });
    /// nodes.insert(2, BasicNode { addr: "127.0.0.1:8081".to_string() });
    /// nodes.insert(3, BasicNode { addr: "127.0.0.1:8082".to_string() });
    /// raft.initialize(nodes).await?;
    /// ```
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn initialize<T>(&self, members: T) -> Result<(), RaftError<C, InitializeError<C>>>
    where T: IntoNodes<C::NodeId, C::Node> + Debug {
        self.management_api().initialize(members).await.into_raft_result()
    }

    /// Provides read-only access to [`RaftState`] through a user-provided function.
    ///
    /// The function `func` is applied to the current [`RaftState`]. The result of this function,
    /// of type `V`, is returned wrapped in `Result<V, Fatal<C>>`. `Fatal` error will be
    /// returned if failed to receive a reply from `RaftCore`.
    ///
    /// A `Fatal` error is returned if:
    /// - Raft core task is stopped normally.
    /// - Raft core task is panicked due to programming error.
    /// - Raft core task is encountered a storage error.
    ///
    /// Example for getting the current committed log id:
    /// ```ignore
    /// let committed = my_raft.with_raft_state(|st| st.committed).await?;
    /// ```
    pub async fn with_raft_state<F, V>(&self, func: F) -> Result<V, Fatal<C>>
    where
        F: FnOnce(&RaftState<C>) -> V + OptionalSend + 'static,
        V: OptionalSend + 'static,
    {
        let (tx, rx) = C::oneshot();

        self.external_request(|st| {
            let result = func(st);
            if let Err(_err) = tx.send(result) {
                tracing::error!("{}: to-Raft tx send error", func_name!());
            }
        })
        .await?;

        match rx.await {
            Ok(res) => Ok(res),
            Err(err) => {
                tracing::error!("{}: rx recv error: {}", func_name!(), err);
                let fatal = self.inner.get_core_stop_error().await;
                Err(fatal)
            }
        }
    }

    /// Send a request to the Raft core loop in a fire-and-forget manner.
    ///
    /// This method returns immediately after sending the message to the Raft core loop,
    /// without waiting for the request to be executed. The returned `Result` indicates
    /// whether the message was successfully sent, not whether the request was executed.
    ///
    /// The request functor will be called with an immutable reference to the [`RaftState`]
    /// and serialized with other Raft core loop processing (e.g., client requests
    /// or general state changes).
    ///
    /// If a response is required, then the caller can store the sender of a one-shot channel
    /// in the closure of the request functor, which can then be used to send the response
    /// asynchronously.
    ///
    /// Returns a `Fatal` error if:
    /// - Raft core task is stopped normally.
    /// - Raft core task is panicked due to programming error.
    /// - Raft core task is encountered a storage error.
    pub async fn external_request<F>(&self, req: F) -> Result<(), Fatal<C>>
    where F: FnOnce(&RaftState<C>) + OptionalSend + 'static {
        let req: BoxOnce<'static, RaftState<C>> = Box::new(req);
        self.inner.send_msg(RaftMsg::WithRaftState { req }).await
    }

    /// Shutdown this Raft node.
    ///
    /// It sends a shutdown signal and waits until `RaftCore` returns.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Gracefully shutdown the Raft node
    /// raft.shutdown().await?;
    /// ```
    pub async fn shutdown(&self) -> Result<(), JoinErrorOf<C>> {
        if let Some(tx) = self.inner.tx_shutdown.lock().unwrap().take() {
            // A failure to send means the RaftCore is already shutdown. Continue to check the task
            // return value.
            let send_res = tx.send(());
            tracing::info!("sending shutdown signal to RaftCore, sending res: {:?}", send_res);
        }
        self.inner.join_core_task().await;
        if let Some(join_handle) = self.inner.tick_handle.shutdown() {
            join_handle.await.ok();
        }

        // TODO(xp): API change: replace `JoinError` with `Fatal`,
        //           to let the caller know the return value of RaftCore task.
        Ok(())
    }

    /// Provides mutable access to [`RaftStateMachine`] through a user-provided function.
    ///
    /// The function `func` is applied to the current [`RaftStateMachine`]. The result of this
    /// function, of type `V`, is returned wrapped in `Result<V, Fatal<C>>`.
    ///
    /// A `Fatal` error is returned if:
    /// - Raft core task is stopped normally.
    /// - Raft core task is panicked due to programming error.
    /// - Raft core task is encountered a storage error.
    ///
    /// Example for getting the last applied log id from SM(assume there is `last_applied()` method
    /// provided):
    ///
    /// ```rust,ignore
    /// let last_applied_log_id = my_raft.with_state_machine(|sm| {
    ///     async move { sm.last_applied().await }
    /// }).await?;
    /// ```
    #[since(version = "0.10.0")]
    pub async fn with_state_machine<F, V>(&self, func: F) -> Result<V, Fatal<C>>
    where
        SM: OptionalSend + 'static,
        F: FnOnce(&mut SM) -> BoxFuture<V> + OptionalSend + 'static,
        V: OptionalSend + 'static,
    {
        let (tx, rx) = C::oneshot();

        self.external_state_machine_request(|sm| {
            Box::pin(async move {
                let resp = func(sm).await;
                if let Err(_err) = tx.send(resp) {
                    tracing::error!("{}: failed to send response to user tx", func_name!());
                }
            })
        })
        .await?;

        // Use the bounded receive: the state-machine worker owns the responder and can die on its
        // own (e.g. a panic in `func`) while RaftCore keeps running. An unbounded join on the core
        // would then hang forever. `recv_msg` waits only up to `RECV_CORE_STOP_TIMEOUT` before
        // resolving to `Fatal::Stopped`.
        self.inner.recv_msg(rx).await
    }

    /// Send a request to the [`RaftStateMachine`] worker in a fire-and-forget manner.
    ///
    /// This method returns immediately after sending the message to the state machine worker,
    /// without waiting for the request to be executed. The returned `Result` indicates
    /// whether the message was successfully sent, not whether the request was executed.
    ///
    /// The request functor will be called with a mutable reference to the state machine.
    /// The functor returns a [`Future`] because state machine methods are `async`.
    ///
    /// Returns a `Fatal` error if:
    /// - Raft core task is stopped normally.
    /// - Raft core task is panicked due to programming error.
    /// - Raft core task is encountered a storage error.
    #[since(version = "0.10.0")]
    pub async fn external_state_machine_request<F>(&self, req: F) -> Result<(), Fatal<C>>
    where
        SM: OptionalSend + 'static,
        F: FnOnce(&mut SM) -> BoxFuture<()> + OptionalSend + 'static,
    {
        let Some(tx) = self.sm_cmd_tx.upgrade() else {
            return Err(self.inner.get_core_stop_error_bounded().await);
        };

        let sm_cmd = sm::Command::ExternalFunc {
            func: Box::new(move |sm| req(sm)),
        };
        if tx.send(sm_cmd).await.is_err() {
            return Err(self.inner.get_core_stop_error_bounded().await);
        }
        Ok(())
    }
}
