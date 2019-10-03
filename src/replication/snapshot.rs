use std::{io::Read, fs::File, path::PathBuf, time::{Duration, Instant}};

use actix::prelude::*;
use log::{error};
use futures::{
    sink::{Sink},
    sync::{mpsc},
};
use tokio_timer::Delay;

use crate::{
    AppData, AppDataResponse, AppError, NodeId,
    common::DependencyAddr,
    messages::{InstallSnapshotRequest, InstallSnapshotResponse},
    network::RaftNetwork,
    replication::{
        ReplicationStream, RSNeedsSnapshot, RSNeedsSnapshotResponse,
        RSRevertToFollower, RSState, RSUpdateMatchIndex,
    },
    storage::{RaftStorage},
};

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, R, E>> ReplicationStream<D, R, E, N, S> {

    /// Drive the replication stream forward when it is in state `Snapshotting`.
    pub(super) fn drive_state_snapshotting(&mut self, ctx: &mut Context<Self>) {
        let _state = match &mut self.state {
            RSState::Snapshotting(state) => state,
            _ => {
                self.is_driving_state = false;
                return self.drive_state(ctx);
            },
        };

        ctx.spawn(fut::wrap_future(self.raftnode.send(RSNeedsSnapshot))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
            // Flatten inner result.
            .and_then(|res, _, _| fut::result(res))
            // Handle response from Raft node and start streaming over the snapshot.
            .and_then(|res, act, ctx| act.send_snapshot(ctx, res))
            // Transition over to lagging state after snapshot has been sent.
            .and_then(|_, act, ctx| act.transition_to_lagging(ctx))
            // Drive state forward regardless of outcome.
            .then(|res, act, ctx| {
                act.is_driving_state = false;
                act.drive_state(ctx);
                fut::result(res)
            }));
    }

    /// Handle frames from the target node which are in response to an InstallSnapshot RPC request.
    fn handle_install_snapshot_response(&mut self, _: &mut Context<Self>, res: InstallSnapshotResponse) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Check the response term. As long as everything still matches, then we are good to resume.
        if &res.term > &self.term {
            fut::Either::B(fut::wrap_future(self.raftnode.send(RSRevertToFollower{target: self.target, term: res.term}))
                .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftInternal))
                // Ensure an error is returned here, as this was not a successful response.
                .and_then(|_, _, _| fut::err(())))
        } else {
            fut::Either::A(fut::ok(()))
        }
    }

    /// Send the specified snapshot over to the target node.
    fn send_snapshot(&mut self, _: &mut Context<Self>, snap: RSNeedsSnapshotResponse) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // Look up the snapshot on disk.
        let (snap_index, snap_term) = (snap.index, snap.term);
        let pathbuf = PathBuf::from(snap.pointer.path);
        let snap_stream = SnapshotStream::new(self.target, pathbuf, self.config.snapshot_max_chunk_size, self.term, self.id, snap_index, snap_term);

        fut::wrap_stream(snap_stream)
            .and_then(|res, _, _| fut::result(res))
            // Send snapshot RPC frame over to target.
            .and_then(move |rpc, act: &mut Self, _| {
                // Send the RPC. If an error is encountered, cancell the stream.
                fut::wrap_future(act.network.send(rpc))
                    .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftNetwork))
                    // Flatten inner result.
                    .and_then(|res, _, _| fut::result(res).map_err(move |_, _, _| ()))
                    // Handle response from target.
                    .and_then(|res, act, ctx| act.handle_install_snapshot_response(ctx, res))
            })

            // Snapshot pass has finished, handle success or error conditions.
            .finish().then(move |res, act, _| match res {
                // Snapshot installation was successful. Update target to track last index of snapshot.
                Ok(_) => {
                    act.next_index = snap_index + 1;
                    act.match_index = snap_index;
                    act.match_term = snap_term;
                    act.raftnode.do_send(RSUpdateMatchIndex{target: act.target, match_index: snap_index});
                    fut::Either::A(fut::ok(()))
                }
                // If an error was encountered for any reason, delay sending the next snapshot for a few seconds.
                Err(_) => {
                    error!("Snapshot stream finished with an error. Delaying before next iteration.");
                    let delay = Instant::now() + Duration::from_secs(5);
                    fut::Either::B(fut::wrap_future(
                        Delay::new(delay).map_err(|_| ()).then(|res| match res {
                            Ok(_) => Err(()),
                            Err(_) => Err(()),
                        })
                    ))
                }
            })
    }
}

//////////////////////////////////////////////////////////////////////////////////////////////////
// SnapshotStream ////////////////////////////////////////////////////////////////////////////////

/// An async stream of the chunks of a given file.
struct SnapshotStream {
    target: NodeId,
    file: PathBuf,
    offset: u64,
    bufsize: u64,
    term: u64,
    leader_id: NodeId,
    last_included_index: u64,
    last_included_term: u64,
    chan: mpsc::Sender<Result<InstallSnapshotRequest, ()>>,
}

impl SnapshotStream {
    /// Create a new instance.
    pub fn new(
        target: NodeId, file: PathBuf, bufsize: u64, term: u64, leader_id: NodeId, last_included_index: u64, last_included_term: u64,
    ) -> mpsc::Receiver<Result<InstallSnapshotRequest, ()>> {
        let (tx, rx) = mpsc::channel(0);
        let inst = Self{
            target, file, offset: 0, bufsize, term, leader_id,
            last_included_index, last_included_term,
            chan: tx,
        };
        std::thread::spawn(move || inst.run());
        rx
    }

    fn run(mut self) {
        // Open the target snapshot file & get a read on its length.
        let mut chan = self.chan.wait();
        let file_and_len = File::open(&self.file)
            .and_then(|file| {
                file.metadata()
                    .map(|meta| (file, meta.len()))
            });
        let (mut file, filelen) = match file_and_len {
            Ok(data) => data,
            Err(err) => {
                error!("Error opening snapshot file for streaming. {}", err);
                let _ = chan.send(Err(()));
                return;
            }
        };

        loop {
            let remaining = filelen - self.offset;
            let chunksize = if self.bufsize > remaining { remaining } else { self.bufsize };
            let mut data = vec![0u8; chunksize as usize];
            if let Err(err) = file.read_exact(&mut data) {
                error!("Error reading from snapshot file for streaming. {}", err);
                let _ = chan.send(Err(()));
                let _ = chan.close();
                return;
            }

            // Build a new frame for the bytes read.
            let mut frame = InstallSnapshotRequest{
                target: self.target, term: self.term, leader_id: self.leader_id,
                last_included_index: self.last_included_index,
                last_included_term: self.last_included_term,
                offset: self.offset, data, done: false,
            };
            self.offset += chunksize;

            // If this was the last chunk, mark it as so.
            let mut is_done = false;
            if self.offset == filelen {
                frame.done = true;
                is_done = true;
            }

            match chan.send(Ok(frame)) {
                Ok(_) if is_done => {
                    let _ = chan.close();
                    return;
                }
                Ok(_) => continue,
                Err(err) => {
                    error!("Error encountered while reading snapshot chunks. {}", err);
                    let _ = chan.close();
                    return;
                }
            }
        }
    }
}
