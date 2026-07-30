use std::collections::VecDeque;
use std::io;

use dir_transfer::DirFrame;
use dir_transfer::FrameSink;
use dir_transfer::FrameSource;

/// In-memory FIFO transport used by checkpoint transfer tests.
#[derive(Default)]
pub(super) struct QueueTransport {
    /// Frames waiting to be consumed by the receiving side.
    frames: VecDeque<DirFrame>,
}

impl QueueTransport {
    pub(super) fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl FrameSink for QueueTransport {
    async fn send_frame(&mut self, frame: DirFrame) -> io::Result<()> {
        self.frames.push_back(frame);
        Ok(())
    }
}

impl FrameSource for QueueTransport {
    async fn recv_frame(&mut self) -> io::Result<DirFrame> {
        self.frames
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "transport closed"))
    }
}
