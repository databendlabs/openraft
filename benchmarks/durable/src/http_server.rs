use std::collections::BTreeMap;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use app_http::App;
use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Method;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper::body::Body;
use hyper::body::Incoming;
use hyper::header;
use hyper::header::HeaderValue;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use openraft::NodeInfo;
use openraft::RaftTypeConfig;
use openraft::alias::MembershipEditOf;
use openraft::impls::ChangeMembers;
use openraft::membership::RaftMembershipEditOps;
use openraft::storage::RaftStateMachine;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::net::TcpListener;

type HandlerFuture = Pin<Box<dyn Future<Output = Response<Full<Bytes>>> + Send>>;
type Handler = Arc<dyn Fn(Bytes) -> HandlerFuture + Send + Sync>;
type Routes = Arc<HashMap<(Method, String), Handler>>;

pub(crate) struct Server<D>
where D: Send + Sync + 'static
{
    app: Arc<D>,
    routes: Routes,
}

impl<D> Server<D>
where D: Send + Sync + 'static
{
    pub(crate) fn new(app: Arc<D>) -> Self {
        Self {
            app,
            routes: Arc::new(HashMap::new()),
        }
    }

    pub(crate) fn post<Req, Resp, Fut, F>(mut self, path: impl Into<String>, handler: F) -> Self
    where
        Req: DeserializeOwned + Send + 'static,
        Resp: Serialize + Send + 'static,
        Fut: Future<Output = Resp> + Send + 'static,
        F: Fn(Arc<D>, Req) -> Fut + Send + Sync + 'static,
    {
        let route = wrap(self.app.clone(), handler);
        Arc::make_mut(&mut self.routes).insert((Method::POST, normalize_path(path)), route);
        self
    }

    pub(crate) fn get<Resp, Fut, F>(mut self, path: impl Into<String>, handler: F) -> Self
    where
        Resp: Serialize + Send + 'static,
        Fut: Future<Output = Resp> + Send + 'static,
        F: Fn(Arc<D>) -> Fut + Send + Sync + 'static,
    {
        let route = wrap(self.app.clone(), move |app, ()| handler(app));
        Arc::make_mut(&mut self.routes).insert((Method::GET, normalize_path(path)), route);
        self
    }

    pub(crate) async fn run(self, addr: impl Into<String>) -> std::io::Result<()> {
        let addr = addr.into();
        let listener = TcpListener::bind(&addr).await?;
        self.run_listener(listener).await
    }

    async fn run_listener(self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let routes = self.routes.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| handle(routes.clone(), req));

                if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                    log::warn!("HTTP connection error: {}", e);
                }
            });
        }
    }
}

impl<C, SM, Data> Server<App<C, SM, Data>>
where
    C: RaftTypeConfig<Node = NodeInfo>,
    C::Membership: From<BTreeMap<C::NodeId, NodeInfo>>,
    MembershipEditOf<C>: From<ChangeMembers<C::NodeId, C::Node>> + RaftMembershipEditOps<C::Membership>,
    SM: RaftStateMachine<C>,
    App<C, SM, Data>: Send + Sync + 'static,
{
    pub(crate) fn add_openraft_routes(self) -> Self {
        self.post("/init", App::<C, SM, Data>::init)
            .post("/add-learner", App::<C, SM, Data>::add_learner)
            .post("/change-membership", App::<C, SM, Data>::change_membership)
            .get("/metrics", App::<C, SM, Data>::metrics)
            .get("/get_linearizer", App::<C, SM, Data>::get_linearizer)
            .post("/write", App::<C, SM, Data>::write)
    }

    pub(crate) fn benchmark_routes(self, backend: &'static str) -> Self {
        self.get("/benchmark/stats", |_app: Arc<App<C, SM, Data>>| async move {
            Ok::<_, String>(crate::benchmark_metrics::report())
        })
        .get("/benchmark/storage", move |_app: Arc<App<C, SM, Data>>| async move {
            Ok::<_, String>(storage(backend))
        })
        .get("/benchmark/config", |app: Arc<App<C, SM, Data>>| async move {
            Ok::<_, String>(app.raft.config().as_ref().clone())
        })
        .post(
            "/benchmark/transfer",
            |app: Arc<App<C, SM, Data>>, target: C::NodeId| async move {
                app.raft.trigger().transfer_leader(target).await.map_err(|error| error.to_string())
            },
        )
    }
}

fn wrap<D, Req, Resp, Fut, F>(app: Arc<D>, handler: F) -> Handler
where
    D: Send + Sync + 'static,
    Req: DeserializeOwned + Send + 'static,
    Resp: Serialize + Send + 'static,
    Fut: Future<Output = Resp> + Send + 'static,
    F: Fn(Arc<D>, Req) -> Fut + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    Arc::new(move |body| {
        let app = app.clone();
        let handler = handler.clone();

        Box::pin(async move {
            let req = match serde_json::from_slice(&body) {
                Ok(req) => req,
                Err(e) => return bad_request(e),
            };

            let resp = handler(app, req).await;
            json_response(&resp)
        })
    })
}

async fn handle(routes: Routes, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let key = (req.method().clone(), req.uri().path().to_string());
    let Some(handler) = routes.get(&key).cloned() else {
        return Ok(error_response(StatusCode::NOT_FOUND, "not found"));
    };

    let received = match req.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => return Ok(error_response(StatusCode::BAD_REQUEST, error)),
    };
    let request_bytes = received.len();
    crate::benchmark_metrics::request(key.1.clone(), request_bytes);
    let body = if key.0 == Method::GET {
        Bytes::from_static(b"null")
    } else {
        received
    };
    let response = handler(body).await;
    let response_bytes = response.body().size_hint().exact().expect("Full body has an exact length");
    crate::benchmark_metrics::response(&key.1, response_bytes as usize);
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

fn json_response<T>(value: &T) -> Response<Full<Bytes>>
where T: Serialize {
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

fn storage(backend: &str) -> serde_json::Value {
    match backend {
        "wal" => serde_json::json!({"backend":"wal", "durability":"sync_WAL_append_flush",
            "state_machine":"volatile_replayed_from_retained_WAL", "snapshots":"disabled"}),
        "rocks" => serde_json::json!({"backend":"rocks", "durability":"sync_RocksDB_log_and_state_machine_batches",
            "state_machine":"persistent_RocksDB", "snapshots":"checkpoint"}),
        _ => unreachable!("fixed backend"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Unserializable;

    impl Serialize for Unserializable {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where S: serde::Serializer {
            Err(serde::ser::Error::custom("response encoding failed"))
        }
    }

    #[tokio::test]
    async fn serialization_failure_is_an_explicit_http_failure() {
        let response = json_response(&Unserializable);
        let status = response.status();
        assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, status);
        let body = response.into_body().collect().await.unwrap();
        let bytes = body.to_bytes();
        assert_eq!(b"response encoding failed".as_slice(), bytes.as_ref());
    }

    #[tokio::test]
    async fn received_requests_are_counted_before_the_handler_finishes() -> std::io::Result<()> {
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;
        crate::benchmark_metrics::enable();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let notifying = entered.clone();
        let released = release.clone();
        let server = Server::new(Arc::new(())).get("/blocked", move |_| {
            let entered = notifying.clone();
            let release = released.clone();
            async move {
                entered.notify_one();
                release.notified().await;
                serde_json::json!({})
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let running = tokio::spawn(server.run_listener(listener));
        let mut client = tokio::net::TcpStream::connect(address).await?;
        client.write_all(b"GET /blocked HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
        entered.notified().await;
        let before = crate::benchmark_metrics::report();
        assert_eq!([1, 0, 0], before["/blocked"]);
        release.notify_one();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await?;
        let body_matches = response.ends_with(b"{}");
        assert!(body_matches);
        let after = crate::benchmark_metrics::report();
        assert_eq!([1, 0, 2], after["/blocked"]);
        running.abort();
        let result = running.await;
        let error = result.unwrap_err();
        assert!(error.is_cancelled());
        Ok(())
    }
}
