# Guide for upgrading from [v0.8](https://github.com/databendlabs/openraft/tree/v0.8.9) to [v0.9](https://github.com/databendlabs/openraft/tree/release-0.9):

[Change log v0.9.0](https://github.com/databendlabs/openraft/blob/release-0.9/change-log.md)


## Major changes:

- Removed Compatibility support for v0.7.

- Add [`AsyncRuntime`][] as an abstraction for async runtime.
  An application can use a different async runtime other than [`tokio`][] by implementing the [`AsyncRuntime`] trait.

- `committed` log id can be optionally saved with [`RaftLogStorage::save_committed()`][].

- New errors: `PayloadTooLarge`: to inform Openraft to split append entries request into smaller ones.

- New API: support linearizable read: [`Raft::ensure_linearizable()`][].

- New API to support generic snapshot data transfer with the following new APIs:

    - [`Raft::get_snapshot()`][]
    - [`Raft::begin_receiving_snapshot()`][]
    - [`Raft::install_full_snapshot()`][]
    - [`RaftNetwork::full_snapshot()`][]

- New feature flags:

    - [`singlethreaded`][]: Run Openraft in a single-threaded environment, i.e., without requiring `Send` bound.
    - [`loosen-follower-log-revert`][]: Allows cleaning a follower's data, useful for testing.
    - [`generic-snapshot-data`][]: Remove `AsyncRead + AsyncWrite` bound from snapshot data. Let the application define its own snapshot data transfer.


## Upgrade for API changes 

The first step for upgrading is to adapt the changes in the API.
Follow the following steps to update your application to pass compilation with v0.9.

- Implementation of `RaftLogReader::get_log_state()` is moved to `RaftLogReader::get_log_state()`.

- Generic types parameters `N, LS, SM` are removed from `Raft<C, N, LS, SM>`.

- `RaftNetwork::send_xxx()` methods are removed, and should be replaced with `RaftNetwork::xxx()`:
  - `RaftNetwork::send_append_entries()` to `RaftNetwork::append_entries()`;
  - `RaftNetwork::send_vote()` to `RaftNetwork::vote()`;
  - `RaftNetwork::send_install_snapshot()` to `RaftNetwork::install_snapshot()`;

- `async` traits in Openraft are declared with [`#[openraft-macros::add_async_trait]`][`openraft-macros`] attribute since 0.9.
  `#[async_trait::async_trait]` are no longer needed when implementing `async` trait.

  For example, upgrade 0.8 async-trait implementation 
  ```ignore
  #[async_trait::async_trait]
  impl RaftNetwork<TypeConfig> for MyNetwork {}
  ```
  
  to 

  ```ignore
  impl RaftNetwork<TypeConfig> for MyNetwork {}
  ```

## Upgrade `RaftTypeConfig`

- Add `AsyncRuntime` to `RaftTypeConfig` to specify the async runtime to use.
  Openraft provides a default implementation: `TokioRuntime`.
  For example:
  ```ignore
  openraft::declare_raft_types!(
    pub TypeConfig:
        // ...
        AsyncRuntime = TokioRuntime
  );
  ```
  
  You can just implement the [`AsyncRuntime`][] trait for your own async runtime and use it in `RaftTypeConfig`.

## Upgrade log storage: save committed log id

In v0.9, `save_committed()` is added to `RaftLogStorage` trait to optionally save the `committed` log id.
For example:
```ignore
impl RaftLogStorage for YourStorage {
    // ...
    fn save_committed(&self, committed: u64) -> Result<(), StorageError> {
        // ...
    }
}
```

If committed log id is saved,
the committed log will be applied to state machine via [`RaftStateMachine::apply()`][] upon startup,
if they are not already applied.

Not implementing `save_committed()` will not affect correctness.

## Upgrade snapshot transmission

In v0.9, an application can use its own snapshot data transfer by enabling [`generic-snapshot-data`][] feature flag.
With this flag enabled, the snapshot data can be arbitrary type
and is no longer required to implement `AsyncRead + AsyncWrite` trait.

To use arbitrary snapshot data, the application needs to:

- Specify the snapshot data type in `RaftTypeConfig`:
  ```ignore
  openraft::declare_raft_types!(
    pub TypeConfig:
        // ...
        SnapshotData = YourSnapshotData
  );
  ```
  
- implement the snapshot transfer in the application's network layer by implementing the [`RaftNetwork`] trait.
  For example:
  ```ignore
  impl RaftNetwork for YourNetwork {
      // ...
      fn full_snapshot(&self, /*...*/) -> Result<_,_> {
          // ...
      }
  }
  ```

  `full_snapshot()` allows for the transfer of snapshot data using any preferred method.
  Openraft provides a default implementation in [`Chunked`][].
  [`Chunked`][] just delegate the transmission to several calls
  to the old chunk-based API `RaftNetwork::install_snapshot()`.
  If you want to minimize the changes in your application,
  just call `Chunked::send_snapshot()` in `full_snapshot()`:
  ```ignore 
  impl RaftNetwork for YourNetwork {
      async fn full_snapshot(&mut self, vote: Vote<C::NodeId>, snapshot: Snapshot<C>, /*...*/
      ) -> Result<SnapshotResponse<C::NodeId>, StreamingError<C, Fatal<C::NodeId>>> {
          let resp = Chunked::send_snapshot(self, vote, snapshot, /*...*/).await?;
          Ok(resp)
      }
  }
  ``` 
  
  Refer to: [the default chunk-based snapshot sending](https://github.com/databendlabs/openraft/blob/2cc7170ffaf87c674e5ca13370402528f8ab3958/openraft/src/network/network.rs#L129)

- On the receiving end,
  when the application finished receiving the snapshot data,
  it should call [`Raft::install_full_snapshot()`][] to install it.

  `Chunked::receive_snapshot` provides a default chunk-based implementation for receiving snapshot data.
  The application can just use it:

  ```ignore
  async fn handle_install_snapshot_request(
      &self,
      req: InstallSnapshotRequest<C>,
  ) -> Result<_, _>
  where C::SnapshotData: AsyncRead + AsyncWrite + AsyncSeek + Unpin,
  {
      let my_vote = self.with_raft_state(|state| *state.vote_ref()).await?;
      let resp = InstallSnapshotResponse { vote: my_vote };

      let finished_snapshot = {
          use crate::network::snapshot_transport::Chunked;
          use crate::network::snapshot_transport::SnapshotTransport;

          let mut streaming = self.snapshot.lock().await;
          Chunked::receive_snapshot(&mut *streaming, self, req).await?
      };

      if let Some(snapshot) = finished_snapshot {
          let resp = self.install_full_snapshot(req_vote, snapshot).await?;
          return Ok(resp.into());
      }
      Ok(resp)
  }
  ```
  
  Refer to: [the default chunk-based snapshot receiving](https://github.com/databendlabs/openraft/blob/c9a463f5ce73d1e7dd66eabfe909fe8d5a087f0e/openraft/src/raft/mod.rs#L447)
  
  Note that the application is responsible for maintaining a streaming state [`Streaming`][]
  during receiving chunks.



[`AsyncRuntime`]:                     `crate::AsyncRuntime`
[`RPCOption`]:                        `crate::network::RPCOption`
[`Chunked`]:                          `crate::network::snapshot_transport::Chunked`
[`Streaming`]:                        `crate::network::snapshot_transport::Streaming`
[`Raft::ensure_linearizable()`]:      `crate::Raft::ensure_linearizable`
[`Raft::get_snapshot()`]:             `crate::Raft::get_snapshot`
[`Raft::begin_receiving_snapshot()`]: `crate::Raft::begin_receiving_snapshot`
[`Raft::install_full_snapshot()`]:    `crate::Raft::install_full_snapshot`

[`RaftNetwork`]:                      `crate::network::RaftNetwork`
[`RaftNetwork::full_snapshot()`]:     `crate::network::v2::RaftNetworkV2::full_snapshot`

[`RaftLogStorage::save_committed()`]: `crate::storage::RaftLogStorage::save_committed`

[`RaftStateMachine::apply()`]:        `crate::storage::RaftStateMachine::apply`

[`singlethreaded`]:              `crate::docs::feature_flags#feature-flag-singlethreaded`
[`loosen-follower-log-revert`]:  `crate::docs::feature_flags#feature-flag-loosen-follower-log-revert`
[`generic-snapshot-data`]:       `crate::docs::feature_flags#feature-flag-generic-snapshot-data`
[`tracing-log`]:                 `crate::docs::feature_flags#feature-flag-tracing-log`

[`openraft-macros`]: https://docs.rs/openraft-macros/latest/openraft_macros/
[`tokio`]: https://tokio.rs/ 
[`monoio`]: https://github.com/bytedance/monoio
