use std::convert::Infallible;
use std::fmt::Display;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Method;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::body::Incoming;
use hyper::header;
use hyper::header::HeaderValue;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use openraft::RaftTypeConfig;
use openraft::errors::decompose::DecomposeResult;
use openraft::storage::RaftStateMachine;
use openraft::type_config::alias::SnapshotDataOf;
use openraft_legacy::prelude::ChunkedSnapshotReceiver;
use openraft_legacy::prelude::SnapshotReceiverFactory;
use serde::Serialize;
use tokio::io::AsyncSeek;
use tokio::io::AsyncWrite;
use tokio::net::TcpListener;

/// The inbound half of the V1 network: serves the three RPCs a peer's
/// [`Network`](crate::Network) sends.
pub struct Server<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    raft: Arc<openraft::Raft<C, SM>>,
}

impl<C, SM> Server<C, SM>
where
    C: RaftTypeConfig,
    SM: SnapshotReceiverFactory<C, SnapshotReceiver = SnapshotDataOf<C, SM>> + 'static,
    // The chunked V1 protocol assembles a snapshot chunk by chunk, so the receiving end
    // needs a file-like snapshot to seek in and write to.
    SnapshotDataOf<C, SM>: AsyncWrite + AsyncSeek + Unpin,
{
    pub fn new(raft: openraft::Raft<C, SM>) -> Self {
        Self { raft: Arc::new(raft) }
    }

    pub async fn run(self, addr: impl Into<String>) -> std::io::Result<()> {
        let addr = addr.into();
        let listener = TcpListener::bind(&addr).await?;

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let raft = self.raft.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| handle(raft.clone(), req));

                if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                    log::warn!("HTTP connection error: {}", e);
                }
            });
        }
    }
}

async fn handle<C, SM>(
    raft: Arc<openraft::Raft<C, SM>>,
    req: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible>
where
    C: RaftTypeConfig,
    SM: SnapshotReceiverFactory<C, SnapshotReceiver = SnapshotDataOf<C, SM>> + 'static,
    SnapshotDataOf<C, SM>: AsyncWrite + AsyncSeek + Unpin,
{
    if req.method() != Method::POST {
        return Ok(error_response(StatusCode::NOT_FOUND, "not found"));
    }

    let path = req.uri().path().to_string();

    let body = match req.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => return Ok(error_response(StatusCode::BAD_REQUEST, e)),
    };

    let response = match handle_raft_rpc(raft, path.as_str(), body).await {
        Ok(resp) | Err(resp) => resp,
    };

    Ok(response)
}

async fn handle_raft_rpc<C, SM>(
    raft: Arc<openraft::Raft<C, SM>>,
    path: &str,
    body: Bytes,
) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>>
where
    C: RaftTypeConfig,
    SM: SnapshotReceiverFactory<C, SnapshotReceiver = SnapshotDataOf<C, SM>>,
    SnapshotDataOf<C, SM>: AsyncWrite + AsyncSeek + Unpin,
{
    // Every route answers with a serialized `Result<Resp, Err>`, splitting the
    // API error out of `RaftError` so the body matches what `Network` decodes.
    // A fatal error leaves the node unusable, so there is nothing to reply with.
    match path {
        "/append" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            Ok(json_response(&raft.append_entries(req).await.decompose().unwrap()))
        }
        "/snapshot" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            Ok(json_response(&raft.install_snapshot(req).await.decompose().unwrap()))
        }
        "/vote" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            Ok(json_response(&raft.vote(req).await.decompose().unwrap()))
        }
        _ => Err(error_response(StatusCode::NOT_FOUND, "not found")),
    }
}

fn json_response<T: Serialize>(value: &T) -> Response<Full<Bytes>> {
    match serde_json::to_vec(value) {
        Ok(body) => response(StatusCode::OK, "application/json", Bytes::from(body)),
        Err(e) => internal_server_error(e),
    }
}

fn bad_request(e: impl Display) -> Response<Full<Bytes>> {
    error_response(StatusCode::BAD_REQUEST, e)
}

fn internal_server_error(e: impl Display) -> Response<Full<Bytes>> {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, e)
}

fn error_response(status: StatusCode, message: impl Display) -> Response<Full<Bytes>> {
    response(status, "text/plain; charset=utf-8", Bytes::from(message.to_string()))
}

fn response(status: StatusCode, content_type: &'static str, body: Bytes) -> Response<Full<Bytes>> {
    let mut resp = Response::new(Full::from(body));
    *resp.status_mut() = status;
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    resp
}
