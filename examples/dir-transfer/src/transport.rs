//! Transport abstraction: how the frames of one session reach the remote peer.

use std::future::Future;
use std::io;

use crate::frame::DirFrame;
use crate::receiver::DirReceiver;
use crate::sender::DirSender;

/// Transmit the frames of one session to the remote peer.
///
/// Implementations choose their own encoding ([`DirFrame`] derives the serde traits) and must
/// deliver frames reliably, in order, without loss or duplication. A transmission failure is
/// reported as an [`io::Error`]; both sides then drop the session.
pub trait FrameSink {
    /// Transmit one frame.
    fn send_frame(&mut self, frame: DirFrame) -> impl Future<Output = io::Result<()>> + Send;
}

/// Receive the frames of one session from the remote peer.
pub trait FrameSource {
    /// Return the next frame of the session.
    ///
    /// A session ends with [`DirFrame::End`]; a transport that runs out of frames before then
    /// reports an error such as [`io::ErrorKind::UnexpectedEof`].
    fn recv_frame(&mut self) -> impl Future<Output = io::Result<DirFrame>> + Send;
}

/// Send one complete session: every frame of `sender` into `sink`.
pub async fn send_dir<S>(mut sender: DirSender, sink: &mut S) -> io::Result<()>
where S: FrameSink {
    while let Some(frame) = sender.next_frame().await? {
        sink.send_frame(frame).await?;
    }
    Ok(())
}

/// Receive one complete session: feed every frame from `source` into `receiver` and call its
/// `finish()` after [`DirFrame::End`].
///
/// No frame is read past `End`, so the transport can carry unrelated traffic afterwards.
pub async fn recv_dir<S>(source: &mut S, mut receiver: DirReceiver) -> io::Result<()>
where S: FrameSource {
    loop {
        let frame = source.recv_frame().await?;
        let is_end = matches!(frame, DirFrame::End);
        receiver.feed(frame).await?;
        if is_end {
            return receiver.finish().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::VecDeque;
    use std::fs;
    use std::io;
    use std::path::Path;

    use tempfile::TempDir;

    use super::FrameSink;
    use super::FrameSource;
    use super::recv_dir;
    use super::send_dir;
    use crate::frame::DirFrame;
    use crate::receiver::DirReceiver;
    use crate::sender::DirSender;

    /// An in-memory transport: sent frames queue up and are received in order.
    #[derive(Default)]
    struct QueueTransport {
        frames: VecDeque<DirFrame>,
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

    /// A sink that fails once `remaining` transmissions are used up.
    struct FailingSink {
        remaining: usize,
    }

    impl FrameSink for FailingSink {
        async fn send_frame(&mut self, _frame: DirFrame) -> io::Result<()> {
            if self.remaining == 0 {
                return Err(io::Error::new(io::ErrorKind::ConnectionReset, "peer gone"));
            }
            self.remaining -= 1;
            Ok(())
        }
    }

    /// Read a directory into `name -> contents`.
    fn dir_contents(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut contents = BTreeMap::new();
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            contents.insert(name, fs::read(entry.path()).unwrap());
        }
        contents
    }

    /// Send a three-file source directory into an in-memory transport.
    async fn sent_session() -> (TempDir, QueueTransport) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"").unwrap();
        fs::write(dir.path().join("b.txt"), b"1234567").unwrap();
        fs::write(dir.path().join("c.bin"), b"abcdefgh").unwrap();

        let sender = DirSender::new(dir.path(), 4).unwrap();
        let mut transport = QueueTransport::default();
        send_dir(sender, &mut transport).await.unwrap();
        (dir, transport)
    }

    #[tokio::test]
    async fn test_round_trip_via_transport() {
        let (source, mut transport) = sent_session().await;
        let target = tempfile::tempdir().unwrap();

        recv_dir(&mut transport, DirReceiver::new(target.path().to_path_buf())).await.unwrap();

        assert_eq!(dir_contents(source.path()), dir_contents(target.path()));
        assert!(transport.frames.is_empty());
    }

    #[tokio::test]
    async fn test_recv_stops_at_end() {
        let (_source, mut transport) = sent_session().await;
        transport.frames.push_back(DirFrame::End);
        let target = tempfile::tempdir().unwrap();

        recv_dir(&mut transport, DirReceiver::new(target.path().to_path_buf())).await.unwrap();

        // The frame after `End` is left in the transport.
        assert_eq!(VecDeque::from([DirFrame::End]), transport.frames);
    }

    #[tokio::test]
    async fn test_transport_closed_before_end() {
        let (_source, mut transport) = sent_session().await;
        transport.frames.pop_back();
        let target = tempfile::tempdir().unwrap();

        let err = recv_dir(&mut transport, DirReceiver::new(target.path().to_path_buf())).await.unwrap_err();

        assert_eq!(io::ErrorKind::UnexpectedEof, err.kind());
    }

    #[tokio::test]
    async fn test_sink_failure_propagates() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f"), b"12345678").unwrap();

        let sender = DirSender::new(dir.path(), 4).unwrap();
        let err = send_dir(sender, &mut FailingSink { remaining: 2 }).await.unwrap_err();

        assert_eq!(io::ErrorKind::ConnectionReset, err.kind());
    }
}
