use std::convert::Infallible as HttpInfallible;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::*;

openraft::declare_raft_types!(C: Node = NodeInfo);

type ReadResult = Result<String, LinearizableReadError<C>>;

#[tokio::test]
async fn redirect_reaches_the_new_leader() {
    let first = listener().await;
    let second = listener().await;
    let first_addr = first.local_addr().unwrap().to_string();
    let second_addr = second.local_addr().unwrap().to_string();
    let calls = Arc::new(AtomicUsize::new(0));
    let success: ReadResult = Ok("new leader value".to_string());
    let body = serde_json::to_string(&success).unwrap();
    let first_task = serve(first, redirect(2, &second_addr), Duration::ZERO, calls.clone());
    let second_task = serve(second, body, Duration::ZERO, calls.clone());
    let client = Client::<C>::new(1, first_addr);
    let actual: ReadResult = client.send_with_forwarding("read", Some(&"key"), 3).await.unwrap();
    let value = actual.unwrap();
    assert_eq!("new leader value", value);
    let count = calls.load(Ordering::SeqCst);
    assert_eq!(2, count);
    first_task.abort();
    second_task.abort();
}

async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}

fn serve(listener: TcpListener, body: String, delay: Duration, calls: Arc<AtomicUsize>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let body = body.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                let service = service_fn(move |_| {
                    let body = body.clone();
                    calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        tokio::time::sleep(delay).await;
                        let response = Response::new(Full::new(Bytes::from(body)));
                        Ok::<_, HttpInfallible>(response)
                    }
                });
                let io = TokioIo::new(stream);
                let _connection = http1::Builder::new().serve_connection(io, service).await;
            });
        }
    })
}

fn redirect(id: u64, addr: &str) -> String {
    let node = NodeInfo::new("raft", addr);
    let error = ForwardToLeader::<C>::new(id, node);
    let result: ReadResult = Err(LinearizableReadError::ForwardToLeader(error));
    serde_json::to_string(&result).unwrap()
}

#[tokio::test]
async fn redirects_follow_leadership_changes_and_preserve_typed_errors() {
    let first = listener().await;
    let second = listener().await;
    let first_addr = first.local_addr().unwrap().to_string();
    let second_addr = second.local_addr().unwrap().to_string();
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let missing: ReadResult = Err(LinearizableReadError::ForwardToLeader(ForwardToLeader::empty()));
    let expected = serde_json::to_value(&missing).unwrap();
    let body = serde_json::to_string(&missing).unwrap();
    let first_task = serve(first, redirect(2, &second_addr), Duration::ZERO, first_calls.clone());
    let second_task = serve(second, body, Duration::ZERO, second_calls.clone());
    let client = Client::<C>::new(1, first_addr);
    let actual: ReadResult = client.send_with_forwarding("read", Some(&"key"), 3).await.unwrap();
    let actual = serde_json::to_value(actual).unwrap();
    assert_eq!(expected, actual);
    let leader = client.leader.lock().unwrap().clone();
    assert_eq!((2, second_addr), leader);
    let calls = first_calls.load(Ordering::SeqCst);
    assert_eq!(1, calls);
    let calls = second_calls.load(Ordering::SeqCst);
    assert_eq!(1, calls);
    first_task.abort();
    second_task.abort();
}

#[tokio::test]
async fn cyclic_leader_hints_stop_at_the_retry_budget() {
    let first = listener().await;
    let second = listener().await;
    let first_addr = first.local_addr().unwrap().to_string();
    let second_addr = second.local_addr().unwrap().to_string();
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let first_body = redirect(2, &second_addr);
    let expected: serde_json::Value = serde_json::from_str(&redirect(1, &first_addr)).unwrap();
    let first_task = serve(first, first_body, Duration::ZERO, first_calls.clone());
    let second_task = serve(second, redirect(1, &first_addr), Duration::ZERO, second_calls.clone());
    let client = Client::<C>::new(1, first_addr);
    let actual: ReadResult = client.send_with_forwarding("read", Some(&"key"), 3).await.unwrap();
    let actual = serde_json::to_value(actual).unwrap();
    assert_eq!(expected, actual);
    let calls = first_calls.load(Ordering::SeqCst);
    assert_eq!(2, calls);
    let calls = second_calls.load(Ordering::SeqCst);
    assert_eq!(2, calls);
    first_task.abort();
    second_task.abort();
}

#[tokio::test]
async fn redirects_share_one_deadline_including_the_response_body() {
    let first = listener().await;
    let second = listener().await;
    let first_addr = first.local_addr().unwrap().to_string();
    let second_addr = second.local_addr().unwrap().to_string();
    let calls = Arc::new(AtomicUsize::new(0));
    let first_task = serve(
        first,
        redirect(2, &second_addr),
        Duration::from_millis(40),
        calls.clone(),
    );
    let success: ReadResult = Ok("value".to_string());
    let body = serde_json::to_string(&success).unwrap();
    let second_task = serve(second, body, Duration::from_millis(40), calls.clone());
    let client = Client::<C>::new(1, first_addr);
    let deadline = Instant::now() + Duration::from_millis(60);
    let actual: Result<ReadResult, _> = client.send_with_forwarding_until("read", Some(&"key"), 3, deadline).await;
    let error = actual.unwrap_err();
    assert!(matches!(error, RPCError::Network(_)));
    first_task.abort();
    second_task.abort();
}

#[derive(Debug)]
struct Unserializable;

impl Serialize for Unserializable {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        Err(serde::ser::Error::custom("encoding failed"))
    }
}

#[tokio::test]
async fn request_encoding_failure_returns_a_transport_error() {
    let client = Client::<C>::new(1, "127.0.0.1:1".to_string());
    let result: Result<ReadResult, _> = client.send("read", Some(&Unserializable)).await;
    let error = result.unwrap_err();
    assert!(matches!(error, RPCError::Network(_)));
    let message = error.to_string();
    assert!(message.contains("encoding failed"));
}

#[tokio::test]
async fn deadline_covers_a_stalled_response_body() {
    let listener = listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 1];
        stream.read_exact(&mut request).await.unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n").await.unwrap();
        std::future::pending::<()>().await;
    });
    let client = Client::<C>::new(1, addr);
    let deadline = Instant::now() + Duration::from_millis(100);
    let actual: Result<ReadResult, _> = client.send_until("read", None::<&()>, deadline).await;
    let error = actual.unwrap_err();
    assert!(matches!(error, RPCError::Network(_)));
    server.abort();
}

#[tokio::test]
async fn failure_status_cannot_be_decoded_as_a_successful_reply() {
    let listener = listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 1];
        stream.read_exact(&mut request).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 16\r\n\r\n{\"Ok\":\"ignored\"}")
            .await
            .unwrap();
    });
    let client = Client::<C>::new(1, addr);
    let actual: Result<ReadResult, _> = client.send("read", None::<&()>).await;
    let error = actual.unwrap_err();
    assert!(matches!(error, RPCError::Network(_)));
    server.await.unwrap();
}

type WriteResult = Result<ClientWriteResponse<C>, ClientWriteError<C>>;

/// The answer of a node that applied a write at log index 5.
fn written() -> String {
    let response = ClientWriteResponse::<C> {
        log_id: openraft::testing::log_id::<C>(1, 1, 5),
        data: "applied".to_string(),
        membership: None,
    };
    let result: WriteResult = Ok(response);
    serde_json::to_string(&result).unwrap()
}

/// The answer of a node that did not append a write, naming node `id` at `addr` as the leader.
fn rejected(id: u64, addr: &str) -> String {
    let node = NodeInfo::new("raft", addr);
    let error = ForwardToLeader::<C>::new(id, node);
    let result: WriteResult = Err(ClientWriteError::ForwardToLeader(error));
    serde_json::to_string(&result).unwrap()
}

/// The answer of a node that appended a write and then dropped it, naming node `id` at `addr` as
/// the leader.
fn discarded(id: u64, addr: &str) -> String {
    let node = NodeInfo::new("raft", addr);
    let error = ForwardToLeader::<C>::new(id, node);
    let result: WriteResult = Err(ClientWriteError::LogEntryDiscarded(error));
    serde_json::to_string(&result).unwrap()
}

#[tokio::test]
async fn write_resends_a_rejected_write_to_the_named_leader() {
    let first = listener().await;
    let second = listener().await;
    let first_addr = first.local_addr().unwrap().to_string();
    let second_addr = second.local_addr().unwrap().to_string();
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let first_task = serve(first, rejected(2, &second_addr), Duration::ZERO, first_calls.clone());
    let second_task = serve(second, written(), Duration::ZERO, second_calls.clone());
    let client = Client::<C>::new(1, first_addr);
    let response = client.write(&"value".to_string()).await.unwrap().unwrap();
    assert_eq!("applied", response.data);
    let calls = first_calls.load(Ordering::SeqCst);
    assert_eq!(1, calls);
    let calls = second_calls.load(Ordering::SeqCst);
    assert_eq!(1, calls);
    first_task.abort();
    second_task.abort();
}

#[tokio::test]
async fn write_returns_a_discarded_write_without_resending_it() {
    let first = listener().await;
    let second = listener().await;
    let first_addr = first.local_addr().unwrap().to_string();
    let second_addr = second.local_addr().unwrap().to_string();
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let first_task = serve(first, discarded(2, &second_addr), Duration::ZERO, first_calls.clone());
    let second_task = serve(second, written(), Duration::ZERO, second_calls.clone());
    let client = Client::<C>::new(1, first_addr);
    let result = client.write(&"value".to_string()).await.unwrap();
    let error = result.unwrap_err();
    assert!(matches!(error, ClientWriteError::LogEntryDiscarded(_)));
    let leader = client.leader.lock().unwrap().clone();
    assert_eq!((2, second_addr), leader);
    let calls = first_calls.load(Ordering::SeqCst);
    assert_eq!(1, calls);
    let calls = second_calls.load(Ordering::SeqCst);
    assert_eq!(0, calls);
    first_task.abort();
    second_task.abort();
}
