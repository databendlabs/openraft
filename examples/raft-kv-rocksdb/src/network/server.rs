use std::convert::Infallible;
use std::fmt::Display;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use dir_transfer::DirWriter;
use dir_transfer::recv_dir;
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
use openraft::raft::TransferLeaderResponse;
use serde::Serialize;
use tokio::net::TcpListener;

use super::snapshot_stream::BodySource;
use crate::Raft;
use crate::TypeConfig;
use crate::typ::*;

/// Serves the Raft RPCs of one node.
pub struct Server {
    raft: Arc<Raft>,

    /// The state machine's directory, where a received checkpoint becomes a new generation.
    state_machine_dir: PathBuf,
}

impl Server {
    pub fn new(raft: Raft, state_machine_dir: PathBuf) -> Self {
        Self {
            raft: Arc::new(raft),
            state_machine_dir,
        }
    }

    pub async fn run(self, addr: impl Into<String>) -> io::Result<()> {
        let addr = addr.into();
        let listener = TcpListener::bind(&addr).await?;
        let server = Arc::new(self);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let server = server.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| handle(server.clone(), req));

                if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::warn!("HTTP connection error: {}", e);
                }
            });
        }
    }

    /// Receive a checkpoint into a new generation directory and install it.
    ///
    /// A transport or protocol failure is an `io::Error`; a failure inside Raft is the
    /// `RaftError` the sender expects in the response body.
    async fn install_snapshot(&self, body: Incoming) -> io::Result<Result<SnapshotResponse, RaftError>> {
        let mut source = BodySource::new(body);
        let (vote, meta): (Vote, SnapshotMeta) = source.recv().await?;

        let received = SnapshotData::new_empty(&self.state_machine_dir)?;
        let writer = DirWriter::new(received.db_path().to_path_buf());
        recv_dir(&mut source, writer).await?;

        let snapshot = Snapshot {
            meta,
            snapshot: received,
        };
        let result = self.raft.install_full_snapshot(vote, snapshot).await.map_err(RaftError::Fatal);
        Ok(result)
    }
}

async fn handle(server: Arc<Server>, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    if req.method() != Method::POST {
        return Ok(error_response(StatusCode::NOT_FOUND, "not found"));
    }

    let path = req.uri().path().to_string();
    let body = req.into_body();

    if path == "/snapshot" {
        let response = match server.install_snapshot(body).await {
            Ok(result) => json_response(&result),
            Err(e) => error_response(StatusCode::BAD_REQUEST, e),
        };
        return Ok(response);
    }

    let body = match body.collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => return Ok(error_response(StatusCode::BAD_REQUEST, e)),
    };

    let response = match handle_json_rpc(&server.raft, path.as_str(), body).await {
        Ok(resp) | Err(resp) => resp,
    };

    Ok(response)
}

async fn handle_json_rpc(raft: &Raft, path: &str, body: Bytes) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
    match path {
        "/append" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            Ok(json_response(&raft.append_entries(req).await))
        }
        "/transfer-leader" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;
            let res: Result<TransferLeaderResponse<TypeConfig>, RaftError> =
                raft.handle_transfer_leader(req).await.map_err(RaftError::Fatal);

            Ok(json_response(&res))
        }
        "/vote" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            Ok(json_response(&raft.vote(req).await))
        }
        "/pre-vote" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            Ok(json_response(&raft.pre_vote(req).await))
        }
        _ => Err(error_response(StatusCode::NOT_FOUND, "not found")),
    }
}

fn json_response<T: Serialize>(value: &T) -> Response<Full<Bytes>> {
    match serde_json::to_vec(value) {
        Ok(body) => response(StatusCode::OK, "application/json", Bytes::from(body)),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

fn bad_request(e: impl Display) -> Response<Full<Bytes>> {
    error_response(StatusCode::BAD_REQUEST, e)
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
