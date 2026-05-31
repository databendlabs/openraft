pub mod api;
pub mod management;

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Method;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::net::TcpListener;

use crate::app::App;

type HandlerFuture = Pin<Box<dyn Future<Output = Result<Vec<u8>, HttpError>> + Send>>;
type Handler = Arc<dyn Fn(Bytes) -> HandlerFuture + Send + Sync>;
type Routes = Arc<HashMap<(Method, String), Handler>>;

#[derive(Debug)]
struct HttpError {
    status: StatusCode,
    message: String,
}

impl HttpError {
    fn bad_request(e: impl Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: e.to_string(),
        }
    }

    fn internal(e: impl Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        }
    }
}

pub struct Server {
    routes: Routes,
}

impl Server {
    pub fn new() -> Self {
        Self {
            routes: Arc::new(HashMap::new()),
        }
    }

    pub fn post<Req, Resp, Fut, F>(mut self, path: impl Into<String>, app: Arc<App>, handler: F) -> Self
    where
        Req: DeserializeOwned + Send + 'static,
        Resp: Serialize + Send + 'static,
        Fut: Future<Output = Resp> + Send + 'static,
        F: Fn(Arc<App>, Req) -> Fut + Send + Sync + 'static,
    {
        let route = wrap_post(app, handler);
        Arc::make_mut(&mut self.routes).insert((Method::POST, normalize_path(path)), route);
        self
    }

    pub fn get<Resp, Fut, F>(mut self, path: impl Into<String>, app: Arc<App>, handler: F) -> Self
    where
        Resp: Serialize + Send + 'static,
        Fut: Future<Output = Resp> + Send + 'static,
        F: Fn(Arc<App>) -> Fut + Send + Sync + 'static,
    {
        let route = wrap_get(app, handler);
        Arc::make_mut(&mut self.routes).insert((Method::GET, normalize_path(path)), route);
        self
    }

    pub async fn run(self, addr: impl Into<String>) -> std::io::Result<()> {
        run_routes(addr, self.routes).await
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

fn wrap_post<Req, Resp, Fut, F>(app: Arc<App>, handler: F) -> Handler
where
    Req: DeserializeOwned + Send + 'static,
    Resp: Serialize + Send + 'static,
    Fut: Future<Output = Resp> + Send + 'static,
    F: Fn(Arc<App>, Req) -> Fut + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    Arc::new(move |body| {
        let app = app.clone();
        let handler = handler.clone();
        let req = serde_json::from_slice(&body);
        let fut = async move {
            let req = req.map_err(HttpError::bad_request)?;
            let resp = handler(app, req).await;
            serde_json::to_vec(&resp).map_err(HttpError::internal)
        };
        Box::pin(fut)
    })
}

fn wrap_get<Resp, Fut, F>(app: Arc<App>, handler: F) -> Handler
where
    Resp: Serialize + Send + 'static,
    Fut: Future<Output = Resp> + Send + 'static,
    F: Fn(Arc<App>) -> Fut + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    Arc::new(move |_body| {
        let app = app.clone();
        let handler = handler.clone();
        let fut = async move {
            let resp = handler(app).await;
            serde_json::to_vec(&resp).map_err(HttpError::internal)
        };
        Box::pin(fut)
    })
}

async fn run_routes(addr: impl Into<String>, routes: Routes) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr.into()).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let routes = routes.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req| handle(routes.clone(), req));

            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                tracing::warn!("HTTP connection error: {}", e);
            }
        });
    }
}

async fn handle(routes: Routes, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let key = (req.method().clone(), req.uri().path().to_string());
    let Some(handler) = routes.get(&key).cloned() else {
        return Ok(error_response(StatusCode::NOT_FOUND, "not found"));
    };

    let body = match req.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(e) => return Ok(error_response(StatusCode::BAD_REQUEST, e)),
    };

    let response = match handler(body).await {
        Ok(body) => json_response(StatusCode::OK, body),
        Err(e) => error_response(e.status, e.message),
    };

    Ok(response)
}

fn normalize_path(path: impl Into<String>) -> String {
    let path = path.into();
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}

fn json_response(status: StatusCode, body: Vec<u8>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Full::from(Bytes::from(body)))
        .unwrap()
}

fn error_response(status: StatusCode, message: impl Display) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::from(Bytes::from(message.to_string())))
        .unwrap()
}
