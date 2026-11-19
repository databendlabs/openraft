use std::io;
use std::io::Cursor;
use std::io::IoSlice;
use std::io::IoSliceMut;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;

/// An in-memory snapshot cursor with independently mutable, shared encoded bytes.
#[derive(Clone, Debug, Default)]
#[openraft::macros::since(version = "0.10.2")]
pub struct SnapshotData {
    data: Arc<Vec<u8>>,
    position: u64,
}

impl SnapshotData {
    /// Create a cursor at byte zero, taking ownership of the buffer.
    #[openraft::macros::since(version = "0.10.2")]
    pub fn new(data: Vec<u8>) -> Self {
        Self::from_shared(Arc::new(data))
    }

    /// Inspect the encoded buffer without changing the cursor or copying bytes.
    #[openraft::macros::since(version = "0.10.2")]
    pub fn get_ref(&self) -> &Vec<u8> {
        &self.data
    }

    /// Obtain an independently mutable buffer, copying bytes only when shared.
    #[openraft::macros::since(version = "0.10.2")]
    pub fn get_mut(&mut self) -> &mut Vec<u8> {
        Arc::make_mut(&mut self.data)
    }

    /// Extract an owned buffer, copying bytes only when another handle retains them.
    #[openraft::macros::since(version = "0.10.2")]
    pub fn into_inner(self) -> Vec<u8> {
        Arc::unwrap_or_clone(self.data)
    }

    /// Return this handle's byte position.
    #[openraft::macros::since(version = "0.10.2")]
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Set this handle's byte position, including positions beyond the buffer's end.
    #[openraft::macros::since(version = "0.10.2")]
    pub fn set_position(&mut self, position: u64) {
        self.position = position;
    }

    pub(crate) fn from_shared(data: Arc<Vec<u8>>) -> Self {
        Self { data, position: 0 }
    }

    pub(crate) fn into_shared(self) -> Arc<Vec<u8>> {
        self.data
    }

    fn with_read<R, F>(&mut self, operation: F) -> R
    where F: FnOnce(&mut Cursor<&[u8]>) -> R {
        let mut cursor = Cursor::new(self.data.as_slice());
        cursor.set_position(self.position);
        let result = operation(&mut cursor);
        self.position = cursor.position();
        result
    }

    fn with_write<R, F>(&mut self, operation: F) -> R
    where F: FnOnce(&mut Cursor<&mut Vec<u8>>) -> R {
        let data = Arc::make_mut(&mut self.data);
        let mut cursor = Cursor::new(data);
        cursor.set_position(self.position);
        let result = operation(&mut cursor);
        self.position = cursor.position();
        result
    }
}

impl Read for SnapshotData {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.with_read(|cursor| cursor.read(buffer))
    }

    fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        self.with_read(|cursor| cursor.read_exact(buffer))
    }

    fn read_vectored(&mut self, buffers: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.with_read(|cursor| cursor.read_vectored(buffers))
    }
}

impl Write for SnapshotData {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.with_write(|cursor| cursor.write(buffer))
    }

    fn write_all(&mut self, buffer: &[u8]) -> io::Result<()> {
        self.with_write(|cursor| cursor.write_all(buffer))
    }

    fn write_vectored(&mut self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
        self.with_write(|cursor| cursor.write_vectored(buffers))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for SnapshotData {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.with_read(|cursor| cursor.seek(position))
    }
}

impl AsyncRead for SnapshotData {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        self.get_mut().with_read(|cursor| {
            let cursor = Pin::new(cursor);
            cursor.poll_read(cx, buffer)
        })
    }
}

impl AsyncWrite for SnapshotData {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buffer: &[u8]) -> Poll<io::Result<usize>> {
        let result = Write::write(self.get_mut(), buffer);
        Poll::Ready(result)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buffers: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let result = Write::write_vectored(self.get_mut(), buffers);
        Poll::Ready(result)
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for SnapshotData {
    fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
        Seek::seek(self.get_mut(), position)?;
        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(self.position))
    }
}

#[cfg(test)]
mod snapshot_data_test;
