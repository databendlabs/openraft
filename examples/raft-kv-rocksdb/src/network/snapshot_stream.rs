//! The wire format of one snapshot transfer: length-prefixed MessagePack messages.
//!
//! The first message is the `(vote, meta)` pair; every following message is one [`DirFrame`].

use std::io;

use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use dir_transfer::DirFrame;
use dir_transfer::DirFrameProducer;
use dir_transfer::FrameSource;
use futures::StreamExt;
use futures::TryStreamExt;
use futures::stream;
use futures::stream::BoxStream;
use http_body_util::BodyDataStream;
use hyper::body::Incoming;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncReadExt;
use tokio_util::io::StreamReader;

/// Largest accepted message: a `Chunk` frame of `MAX_CHUNK_SIZE` bytes plus its envelope.
const MAX_MESSAGE_SIZE: usize = dir_transfer::MAX_CHUNK_SIZE + 1024;

/// Encode `message` as a big-endian `u32` byte count followed by its MessagePack bytes.
fn encode<T>(message: &T) -> io::Result<Bytes>
where T: Serialize {
    let payload = rmp_serde::to_vec(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(payload.len()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut buf = BytesMut::with_capacity(4 + payload.len());
    buf.put_u32(len);
    buf.put_slice(&payload);
    Ok(buf.freeze())
}

/// The request body of one transfer: `start`, then every frame `producer` yields.
///
/// `DirFrameProducer` is pull based, so the body reads the next frame only when the connection has
/// room for it; the receiver's pace limits the sender's.
pub(super) fn request_body<T>(start: &T, producer: DirFrameProducer) -> io::Result<reqwest::Body>
where T: Serialize {
    let start = encode(start)?;
    let frames = stream::try_unfold(producer, |mut producer| async move {
        let Some(frame) = producer.next_frame().await? else {
            return Ok(None);
        };
        let bytes = encode(&frame)?;
        Ok::<_, io::Error>(Some((bytes, producer)))
    });
    let messages = stream::once(async move { Ok::<_, io::Error>(start) }).chain(frames);
    Ok(reqwest::Body::wrap_stream(messages))
}

/// Reads the messages of one transfer from the request body.
pub(super) struct BodySource {
    reader: StreamReader<BoxStream<'static, io::Result<Bytes>>, Bytes>,
}

impl BodySource {
    pub(super) fn new(body: Incoming) -> Self {
        let data = BodyDataStream::new(body).map_err(io::Error::other);
        Self {
            reader: StreamReader::new(data.boxed()),
        }
    }

    /// Read the next message; a body that ends first fails with `UnexpectedEof`.
    pub(super) async fn recv<T>(&mut self) -> io::Result<T>
    where T: DeserializeOwned {
        let len = self.reader.read_u32().await? as usize;
        if len > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("message of {len} bytes exceeds {MAX_MESSAGE_SIZE}"),
            ));
        }

        let mut payload = vec![0; len];
        self.reader.read_exact(&mut payload).await?;
        rmp_serde::from_slice(&payload).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl FrameSource for BodySource {
    async fn recv_frame(&mut self) -> io::Result<DirFrame> {
        self.recv().await
    }
}
