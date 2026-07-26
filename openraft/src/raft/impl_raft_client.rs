//! The read and write API an application calls on its local Raft node.

use openraft_macros::since;

use crate::Raft;
use crate::RaftTypeConfig;
use crate::async_runtime::watch::WatchReceiver;
use crate::base::BoxStream;
use crate::entry::EntryPayload;
use crate::errors::ClientWriteError;
use crate::errors::Fatal;
use crate::errors::LinearizableReadError;
use crate::errors::RaftError;
use crate::errors::into_raft_result::IntoRaftResult;
use crate::raft::ReadPolicy;
use crate::raft::linearizable_read::Linearizer;
use crate::raft::message::ClientWriteResponse;
use crate::raft::message::WriteRequest;
use crate::raft::message::WriteResult;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::WriteResponderOf;

/// Implement the client-facing read and write operations, including linearizable reads and the
/// several ways to propose a write.
impl<C, SM> Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    /// Get the ID of the current leader from this Raft node.
    ///
    /// This method is based on the Raft metrics system which does a good job at staying
    /// up-to-date; however, the `is_leader` method must still be used to guard against stale
    /// reads. This method is perfect for making decisions on where to route client requests.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn current_leader(&self) -> Option<C::NodeId> {
        self.metrics().borrow_watched().current_leader.clone()
    }

    /// Ensures reads performed after this method are linearizable across the cluster
    /// using an explicitly provided policy. This method is just a shorthand for calling
    /// [`get_read_log_id()`](Raft::get_read_log_id) and then calling [Raft::wait].
    ///
    /// This method is just a shorthand for combining calling
    /// [`Raft::get_read_linearizer()`](Self::get_read_linearizer) and
    /// [`Linearizer::try_await_ready()`](Linearizer::try_await_ready), i.e., it is
    /// equivalent to:
    /// ```ignore
    /// my_raft.get_read_linearizer(read_policy).await?.try_await_ready(&my_raft, None).await?;
    /// ```
    ///
    /// To support follower read, i.e., get `read_log_id` on a remote leader then read on local
    /// state machine, see [`Raft::get_read_linearizer`].
    ///
    /// The `read_policy` defines the policy to ensure leadership. See: [`ReadPolicy`].
    ///
    /// Returns:
    /// - `Ok(read_log_id)` on successful confirmation that the node is the leader. `read_log_id`
    ///   represents the log id up to which the state machine has applied to ensure a linearizable
    ///   read.
    /// - `Err(RaftError<LinearizableReadError>)` if fails to assert leadership.
    ///
    /// # Examples
    /// ```ignore
    /// // Use a strict policy for this specific critical read
    /// my_raft.ensure_linearizable(ReadPolicy::ReadIndex).await?;
    ///
    /// // Or use a more performant policy when consistency requirements are less strict
    /// my_raft.ensure_linearizable(ReadPolicy::LeaseRead).await?;
    ///
    /// // Then proceed with the state machine read
    /// ```
    /// Read more about how it works: [Read Operation](crate::docs::protocol::read)
    #[since(version = "0.9.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn ensure_linearizable(
        &self,
        read_policy: ReadPolicy,
    ) -> Result<Option<LogIdOf<C>>, RaftError<C, LinearizableReadError<C>>> {
        let linearizer = self.app_api().get_read_linearizer(read_policy).await.into_raft_result()?;

        // Safe unwrap: it never times out.
        let state = linearizer.await_ready(self).await?;
        Ok(Some(state.read_log_id().clone()))
    }

    /// Legacy method that returns log IDs directly. Use
    /// [`Raft::get_read_linearizer`] instead.
    ///
    /// This method extracts log IDs from a [`Linearizer`] and returns them as a tuple.
    /// **For new code, use [`Raft::get_read_linearizer`]** which provides a better API.
    ///
    /// See [`Raft::get_read_linearizer`] for full documentation.
    #[since(version = "0.9.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn get_read_log_id(
        &self,
        read_policy: ReadPolicy,
    ) -> Result<(Option<LogIdOf<C>>, Option<LogIdOf<C>>), RaftError<C, LinearizableReadError<C>>> {
        let linearizer = self.app_api().get_read_linearizer(read_policy).await.into_raft_result()?;

        let read_log_id = linearizer.read_log_id();
        let applied = linearizer.applied();

        Ok((Some(read_log_id.clone()), applied.cloned()))
    }

    /// Ensures this node is leader and returns a [`Linearizer`] to linearize reads.
    ///
    /// This method confirms leadership and provides the necessary information to linearize reads
    /// across the cluster. The leadership is ensured by sending heartbeats or by lease according
    /// to the specified policy. See: [`ReadPolicy`].
    ///
    /// Returns:
    /// - `Ok(Linearizer<C>)` on successful confirmation that the node is the leader. The
    ///   [`Linearizer`] contains the `read_log_id` up to which the state machine should apply to
    ///   linearize reads, and the last `applied` log id.
    /// - `Err(RaftError<LinearizableReadError>)` if this node fails to ensure its leadership, for
    ///   example, it detects a higher term, or fails to communicate with a quorum.
    ///
    /// Once returned, the caller should block until the state machine to apply up to `read_log_id`
    /// using [`Linearizer::try_await_ready`].
    ///
    /// # Examples
    /// ```ignore
    /// let linearizer = my_raft.get_read_linearizer(ReadPolicy::ReadIndex).await?;
    /// let _ = linearizer.try_await_ready(&my_raft, None).await?.unwrap();
    ///
    /// // Following read from state machine is linearized across the cluster
    /// let val = my_raft.with_state_machine(|sm| { sm.read("foo") }).await?;
    /// ```
    ///
    /// # Follower Read
    ///
    /// For follower reads, obtain the `read_log_id` from the leader via application-defined RPC,
    /// then use [`Linearizer::try_await_ready`] to wait for local state machine to catch
    /// up.
    ///
    /// ```ignore
    /// // Application defined RPC to get the `read_log_id` from the remote leader
    /// let leader_id = my_raft.current_leader().await?.unwrap();
    /// let linearizer = my_app_rpc.get_read_linearizer(leader_id, ReadPolicy::ReadIndex).await?;
    ///
    /// // Block waiting local state machine to apply up to to the `read_log_id`
    /// let _ = linearizer.try_await_ready(&my_raft, None).await?.unwrap();
    ///
    /// // Following read from state machine is linearized across the cluster
    /// let val = my_raft.with_state_machine(|sm| { sm.read("foo") }).await?;
    /// ```
    ///
    /// See: [Read Operation](crate::docs::protocol::read)
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn get_read_linearizer(
        &self,
        read_policy: ReadPolicy,
    ) -> Result<Linearizer<C>, RaftError<C, LinearizableReadError<C>>> {
        self.app_api().get_read_linearizer(read_policy).await.into_raft_result()
    }

    /// Submit a mutating client request to Raft to update the state of the system (§5.1).
    ///
    /// It will be appended to the log, committed to the cluster, and then applied to the
    /// application state machine. The result of applying the request to the state machine will
    /// be returned as the response from this method.
    ///
    /// Our goal for Raft is to implement linearizable semantics. If the leader crashes after
    /// committing a log entry but before responding to the client, the client may retry the
    /// command with a new leader, causing it to be executed a second time. As such, clients
    /// should assign unique serial numbers to every command. Then, the state machine should
    /// track the latest serial number processed for each client, along with the associated
    /// response. If it receives a command whose serial number has already been executed, it
    /// responds immediately without re-executing the request (§8). The
    /// [`RaftStateMachine::apply`] method is the perfect place to implement
    /// this.
    ///
    /// These are application specific requirements, and must be implemented by the application
    /// which is being built on top of Raft.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Submit a write request
    /// let request = MyAppData { key: "foo".to_string(), value: "bar".to_string() };
    /// let response = raft.client_write(request).await?;
    /// println!("Applied at log index: {:?}", response.log_id);
    /// ```
    #[tracing::instrument(level = "debug", skip(self, app_data))]
    pub async fn client_write(
        &self,
        app_data: C::D,
    ) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>> {
        self.app_api().client_write(EntryPayload::Normal(app_data)).await.into_raft_result()
    }

    /// Write a blank log entry to the Raft log.
    ///
    /// A blank entry contains no application data and is typically used to:
    /// - Commit entries from previous terms when a new leader is elected
    /// - Advance the commit index without any state machine changes
    /// - Act as a barrier to ensure all previous entries are committed
    ///
    /// Returns when the blank entry has been applied to the state machine.
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn write_blank(&self) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>> {
        self.app_api().client_write(EntryPayload::Blank).await.into_raft_result()
    }

    /// Submit a mutating client request to Raft to update the state machine, returns an application
    /// defined response receiver [`Responder::Receiver`].
    ///
    /// `_ff` means fire and forget.
    ///
    /// It is same as [`Self::client_write`] but does not wait for the response.
    #[since(version = "0.10.0", date = "2025-10-27", change = "add responder arg")]
    #[since(version = "0.10.0")]
    pub async fn client_write_ff(
        &self,
        app_data: C::D,
        responder: Option<WriteResponderOf<C>>,
    ) -> Result<(), Fatal<C>> {
        self.app_api().client_write_ff(EntryPayload::Normal(app_data), responder).await
    }

    /// Write multiple application data payloads in a single batch.
    ///
    /// Returns a stream that yields each result in submission order.
    /// This is more efficient than calling [`client_write()`](Self::client_write) multiple times
    /// as it sends all payloads in a single message to the Raft core.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use futures_util::TryStreamExt;
    ///
    /// let mut stream = raft.client_write_many([data1, data2, data3]).await?;
    ///
    /// // try_next() extracts Fatal error, result is WriteResult
    /// while let Some(result) = stream.try_next().await? {
    ///     match result {
    ///         Ok(response) => println!("Applied at log index: {:?}", response.log_id),
    ///         Err(forward_err) => eprintln!("Forward to leader: {:?}", forward_err),
    ///     }
    /// }
    /// ```
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn client_write_many(
        &self,
        app_data: impl IntoIterator<Item = C::D>,
    ) -> Result<BoxStream<'static, Result<WriteResult<C>, Fatal<C>>>, Fatal<C>> {
        self.app_api().client_write_many(app_data.into_iter().map(EntryPayload::Normal)).await
    }

    /// Submit a write request to Raft.
    ///
    /// Returns a [`WriteRequest`] builder. Fire-and-forget by default;
    /// use [`.responder()`] for results, [`.with_leader()`] for conditional writes.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use openraft::impls::ProgressResponder;
    ///
    /// // Fire-and-forget
    /// raft.write(my_data).await?;
    ///
    /// // With responder
    /// let (responder, rx) = ProgressResponder::complete_only();
    /// raft.write(my_data).responder(responder).await?;
    /// let result = rx.await??;
    ///
    /// // Conditional write (fails if leader changed)
    /// let leader_id = raft.as_leader()?.to_committed_leader_id();
    /// raft.write(my_data)
    ///     .with_leader(leader_id)
    ///     .responder(responder)
    ///     .await?;
    /// ```
    ///
    /// [`.responder()`]: WriteRequest::responder
    /// [`.with_leader()`]: WriteRequest::with_leader
    #[since(version = "0.10.0")]
    pub fn write(&self, app_data: C::D) -> WriteRequest<'_, C> {
        WriteRequest {
            inner: &self.inner,
            app_data,
            responder: None,
            expected_leader: None,
        }
    }
}
