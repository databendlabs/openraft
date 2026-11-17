//! Explicit process-benchmark fault control. Normal example nodes do not install it.

use std::collections::BTreeSet;
use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::OnceLock;

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::Request;
use hyper::Response;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::watch;

static ACTIVE: OnceLock<Arc<Faults>> = OnceLock::new();

/// Optional outgoing Raft partition gate, including requests already in flight.
#[openraft::macros::since(version = "0.10.1")]
pub struct Faults {
    blocked: watch::Sender<Partition>,
}

#[derive(Default)]
struct Partition {
    blocked: BTreeSet<String>,
    generation: u64,
    last_blocked: std::collections::BTreeMap<String, u64>,
}

impl Faults {
    /// Install one gate for an explicitly controlled, single-node example process.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn install() -> io::Result<Arc<Self>> {
        let (blocked, _) = watch::channel(Partition::default());
        let faults = Arc::new(Self { blocked });
        ACTIVE
            .set(faults.clone())
            .map_err(|_| io::Error::new(io::ErrorKind::AlreadyExists, "benchmark fault gate already installed"))?;
        Ok(faults)
    }

    /// Return the process's explicitly installed gate, if any.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn active() -> Option<Arc<Self>> {
        ACTIVE.get().cloned()
    }

    /// Replace target node IDs whose outgoing requests should fail.
    #[openraft::macros::since(version = "0.10.1")]
    pub fn partition(&self, blocked: BTreeSet<String>) {
        self.blocked.send_modify(|state| {
            state.generation += 1;
            for target in &blocked {
                state.last_blocked.insert(target.clone(), state.generation);
            }
            state.blocked = blocked;
        });
    }

    /// Cancel the entire operation if its link becomes blocked.
    #[openraft::macros::since(version = "0.10.1")]
    pub async fn run<F, T>(&self, target: &str, operation: F) -> io::Result<T>
    where F: Future<Output = T> {
        let mut changes = self.blocked.subscribe();
        let generation = changes.borrow().generation;
        tokio::pin!(operation);
        loop {
            self.check(target, generation)?;
            tokio::select! {
                result = &mut operation => { self.check(target, generation)?; return Ok(result); },
                changed = changes.changed() => changed.map_err(io::Error::other)?,
            }
        }
    }

    fn check(&self, target: &str, generation: u64) -> io::Result<()> {
        let state = self.blocked.borrow();
        let target_epoch = state.last_blocked.get(target).copied().unwrap_or(0);
        let wildcard_epoch = state.last_blocked.get("*").copied().unwrap_or(0);
        if state.blocked.contains(target)
            || state.blocked.contains("*")
            || target_epoch > generation
            || wildcard_epoch > generation
        {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                format!("benchmark partition to {target}"),
            ));
        }
        Ok(())
    }

    /// Serve POST /partition with a JSON target-ID list, and GET /status.
    #[openraft::macros::since(version = "0.10.1")]
    pub async fn serve(self: Arc<Self>, addr: String) -> io::Result<()> {
        let listener = TcpListener::bind(addr).await?;
        loop {
            let (stream, _) = listener.accept().await?;
            let faults = self.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| faults.clone().handle(request));
                let result = http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
                if let Err(error) = result {
                    log::error!("benchmark control connection: {error}");
                }
            });
        }
    }

    async fn handle(self: Arc<Self>, request: Request<Incoming>) -> io::Result<Response<Full<Bytes>>> {
        let status = request.uri().path() == "/status" && request.method() == hyper::Method::GET;
        let update = request.uri().path() == "/partition" && request.method() == hyper::Method::POST;
        if !status && !update {
            return Ok(reply(404, "unknown benchmark control route".into()));
        }
        if update {
            let body = request.into_body().collect().await.map_err(io::Error::other)?;
            let blocked = match serde_json::from_slice::<BTreeSet<String>>(&body.to_bytes()) {
                Ok(blocked) => blocked,
                Err(error) => return Ok(reply(400, error.to_string())),
            };
            self.partition(blocked);
        }
        let body = serde_json::to_string(&self.blocked.borrow().blocked).map_err(io::Error::other)?;
        Ok(reply(200, body))
    }
}

fn reply(status: u16, body: String) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = hyper::StatusCode::from_u16(status).expect("fixed status");
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn partition_cancels_inflight_requests_and_heal_allows_new_work() -> io::Result<()> {
        let (blocked, _) = watch::channel(Partition::default());
        let gate = Arc::new(Faults { blocked });
        let running = gate.clone();
        let (entered, started) = tokio::sync::oneshot::channel();
        let request = tokio::spawn(async move {
            running
                .run("3", async move {
                    entered.send(()).unwrap();
                    std::future::pending::<()>().await;
                })
                .await
        });
        started.await.map_err(io::Error::other)?;
        gate.partition(BTreeSet::from(["3".to_string()]));
        let result = request.await.map_err(io::Error::other)?;
        let error = result.unwrap_err();
        assert_eq!(io::ErrorKind::ConnectionAborted, error.kind());
        gate.partition(BTreeSet::new());
        let result = gate.run("3", std::future::ready(42)).await?;
        assert_eq!(42, result);
        Ok(())
    }
}

#[cfg(test)]
mod epoch_tests {
    use super::*;

    #[tokio::test]
    async fn block_then_heal_cancels_old_work_without_cancelling_other_links() -> io::Result<()> {
        let (blocked, _) = watch::channel(Partition::default());
        let gate = Faults { blocked };
        let (done, result) = tokio::sync::oneshot::channel();
        let mut operation = Box::pin(gate.run("3", result));
        let pending = std::future::poll_fn(|cx| {
            let pending = operation.as_mut().poll(cx).is_pending();
            std::task::Poll::Ready(pending)
        })
        .await;
        assert!(pending);
        gate.partition(BTreeSet::from(["3".to_string()]));
        gate.partition(BTreeSet::new());
        done.send(7).unwrap();
        let result = operation.await;
        let error = result.unwrap_err();
        assert_eq!(io::ErrorKind::ConnectionAborted, error.kind());
        let new = gate.run("3", std::future::ready(42)).await?;
        assert_eq!(42, new);
        let unaffected = gate.run("2", std::future::ready(43)).await?;
        assert_eq!(43, unaffected);
        Ok(())
    }
}
