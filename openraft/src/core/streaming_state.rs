// use std::marker::PhantomData;

// use futures::Sink;
// use futures::SinkExt;
// use tokio::io::AsyncWrite;
// use tokio::io::AsyncWriteExt;

// use crate::raft::InstallSnapshotRequest;
// use crate::ErrorSubject;
// use crate::ErrorVerb;
// use crate::RaftTypeConfig;
// use crate::SnapshotId;
// use crate::StorageError;

// /// The Raft node is streaming in a snapshot from the leader.
// pub(crate) struct StreamingState<C: RaftTypeConfig, SD> {
//     /// The offset of the last byte written to the snapshot.
//     pub(crate) offset: u64,
//     /// The ID of the snapshot being written.
//     pub(crate) snapshot_id: SnapshotId,
//     /// A handle to the snapshot writer.
//     pub(crate) snapshot_data: SD,

//     _p: PhantomData<C>,
// }

// impl<C: RaftTypeConfig, SD> StreamingState<C, SD>
// where SD: Sink<C::SD, Error = std::io::Error> + Unpin
// {
//     pub(crate) fn new(snapshot_id: SnapshotId, snapshot_data: Box<SD>) -> Self {
//         Self {
//             offset: 0,
//             snapshot_id,
//             snapshot_data,
//             _p: Default::default(),
//         }
//     }
// }
