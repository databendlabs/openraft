use std::convert::Infallible;
use std::fmt::Display;
use std::io::Cursor;
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
use openraft::NodeInfo;
use openraft::RaftTypeConfig;
use openraft::Snapshot;
use openraft::errors::RaftError;
use openraft::raft::SnapshotResponse;
use openraft::raft::TransferLeaderResponse;
use serde::Serialize;
use tokio::net::TcpListener;

pub struct Server<C, SM>
where C: RaftTypeConfig<Node = NodeInfo, SnapshotData = Cursor<Vec<u8>>>
{
    raft: Arc<openraft::Raft<C, SM>>,
}

impl<C, SM> Server<C, SM>
where
    C: RaftTypeConfig<Node = NodeInfo, SnapshotData = Cursor<Vec<u8>>>,
    SM: 'static,
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
    C: RaftTypeConfig<Node = NodeInfo, SnapshotData = Cursor<Vec<u8>>>,
    SM: 'static,
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
    C: RaftTypeConfig<Node = NodeInfo, SnapshotData = Cursor<Vec<u8>>>,
{
    match path {
        "/append" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            json_response(&raft.append_entries(req).await)
        }
        "/snapshot" => {
            let (vote, meta, data) = serde_json::from_slice(&body).map_err(bad_request)?;
            let snapshot = Snapshot {
                meta,
                snapshot: Cursor::new(data),
            };
            let res: Result<SnapshotResponse<C>, RaftError<C>> =
                raft.install_full_snapshot(vote, snapshot).await.map_err(RaftError::Fatal);

            json_response(&res)
        }
        "/transfer-leader" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;
            let res: Result<TransferLeaderResponse<C>, RaftError<C>> =
                raft.handle_transfer_leader(req).await.map_err(RaftError::Fatal);

            json_response(&res)
        }
        "/vote" => {
            let req = serde_json::from_slice(&body).map_err(bad_request)?;

            json_response(&raft.vote(req).await)
        }
        _ => Err(error_response(StatusCode::NOT_FOUND, "not found")),
    }
}

fn json_response<T: Serialize>(value: &T) -> Result<Response<Full<Bytes>>, Response<Full<Bytes>>> {
    let body = serde_json::to_vec(value).map_err(internal_server_error)?;

    Ok(response(StatusCode::OK, "application/json", Bytes::from(body)))
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
