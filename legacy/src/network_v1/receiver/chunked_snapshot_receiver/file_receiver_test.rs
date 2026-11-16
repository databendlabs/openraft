use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use futures::Stream;
use openraft::Config;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::RaftNetworkFactory;
use openraft::RaftSnapshotBuilder;
use openraft::errors::RPCError;
use openraft::errors::ReplicationClosed;
use openraft::errors::StreamingError;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendResult;
use openraft::raft::SnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::LogIdOf;
use openraft::type_config::alias::SnapshotMetaOf;
use openraft::type_config::alias::SnapshotOf;
use openraft::type_config::alias::StoredMembershipOf;
use openraft::type_config::alias::VoteOf;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::network_v1::ChunkedSnapshotReceiver;
use crate::network_v1::InstallSnapshotRequest;
use crate::network_v1::SnapshotMeta;
use crate::network_v1::SnapshotReceiverFactory;

openraft::declare_raft_types!(C: Node = ());

struct FileStateMachine {
    receive_dir: PathBuf,
    installed: Option<(SnapshotMetaOf<C>, Vec<u8>)>,
}

impl SnapshotReceiverFactory<C> for FileStateMachine {
    type SnapshotReceiver = File;

    async fn begin_receiving_snapshot(&mut self) -> io::Result<File> {
        tokio::fs::create_dir_all(&self.receive_dir).await?;
        let file = tempfile::tempfile_in(&self.receive_dir)?;
        Ok(File::from_std(file))
    }
}

impl RaftStateMachine<C> for FileStateMachine {
    type SnapshotData = File;
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> io::Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>)> {
        let meta = match &self.installed {
            Some((meta, _)) => meta.clone(),
            None => SnapshotMetaOf::<C>::default(),
        };
        Ok((meta.last_log_id, meta.last_membership))
    }

    async fn apply<S>(&mut self, _entries: S) -> io::Result<()>
    where S: Stream<Item = io::Result<EntryResponder<C>>> + Unpin + OptionalSend {
        unreachable!("this follower receives only snapshot chunks")
    }

    async fn get_snapshot_builder(&mut self) -> Self {
        unreachable!("this follower does not build snapshots")
    }

    async fn install_snapshot(&mut self, meta: &SnapshotMetaOf<C>, mut snapshot: File) -> io::Result<()> {
        let mut contents = Vec::new();
        snapshot.read_to_end(&mut contents).await?;
        self.installed = Some((meta.clone(), contents));
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> io::Result<Option<SnapshotOf<C, File>>> {
        Ok(None)
    }
}

impl RaftSnapshotBuilder<C> for FileStateMachine {
    type SnapshotData = File;

    async fn build_snapshot(&mut self) -> io::Result<SnapshotOf<C, File>> {
        unreachable!("this follower does not build snapshots")
    }
}

struct UnusedNetwork;

impl RaftNetworkFactory<C> for UnusedNetwork {
    type Network = Self;

    async fn new_client(&mut self, _target: u64, _node: &()) -> Self {
        unreachable!("this follower does not send RPCs")
    }
}

impl RaftNetworkV2<C> for UnusedNetwork {
    type SnapshotData = File;

    async fn append_entries(
        &mut self,
        _rpc: AppendEntriesRequest<C>,
        _option: RPCOption,
    ) -> Result<AppendResult<C>, RPCError<C>> {
        unreachable!("this follower does not send RPCs")
    }

    async fn vote(&mut self, _rpc: VoteRequest<C>, _option: RPCOption) -> Result<VoteResponse<C>, RPCError<C>> {
        unreachable!("this follower does not send RPCs")
    }

    async fn full_snapshot(
        &mut self,
        _vote: VoteOf<C>,
        _snapshot: SnapshotOf<C, File>,
        _cancel: impl Future<Output = ReplicationClosed> + OptionalSend + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<C>, StreamingError<C>> {
        unreachable!("this follower does not send RPCs")
    }
}

#[test]
fn fresh_file_receiver_installs_all_chunks_from_the_start() {
    C::run(async {
        let root = tempfile::tempdir().unwrap().keep();
        let receive_dir = root.join("fresh").join("receiving");
        let config = Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            ..Config::default()
        };
        let state_machine = FileStateMachine {
            receive_dir: receive_dir.clone(),
            installed: None,
        };
        let raft = Raft::new(
            2,
            Arc::new(config),
            UnusedNetwork,
            log_mem::LogStore::default(),
            state_machine,
        )
        .await
        .unwrap();
        let meta = SnapshotMetaOf::<C> {
            last_log_id: Some(openraft::testing::log_id::<C>(1, 1, 5)),
            last_membership: StoredMembershipOf::<C>::default(),
        };
        let transfer_meta = SnapshotMeta::new(meta.clone(), "../opaque-peer-id".to_string());
        let chunks = [b"first chunk\0".as_slice(), b"middle chunk\n", b"final chunk"];
        let mut offset = 0;
        let exists = receive_dir.exists();
        assert!(!exists);

        tracing::info!("partial chunks create a fresh receive directory without publishing state");
        for (index, chunk) in chunks.iter().enumerate() {
            let done = index == chunks.len() - 1;
            let request = InstallSnapshotRequest::<C> {
                vote: VoteOf::<C>::new_committed(1, 1),
                meta: transfer_meta.clone(),
                offset,
                data: chunk.to_vec(),
                done,
            };
            raft.install_snapshot(request).await.unwrap();
            offset += chunk.len() as u64;
            if !done {
                let installed = raft.with_state_machine(|sm| Box::pin(async { sm.installed.clone() })).await.unwrap();
                assert_eq!(None, installed);
            }
        }

        tracing::info!("the final chunk hands the complete readable file to installation");
        {
            let installed = raft.with_state_machine(|sm| Box::pin(async { sm.installed.clone() })).await.unwrap();
            let expected = Some((meta, chunks.concat()));
            assert_eq!(expected, installed);
            let exists = receive_dir.is_dir();
            assert!(exists);
        }
        raft.shutdown().await.unwrap();
        drop(raft);
        std::fs::remove_dir_all(root).unwrap();
    });
}
