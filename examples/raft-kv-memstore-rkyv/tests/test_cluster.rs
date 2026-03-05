#![allow(clippy::uninlined_format_args)]

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use openraft::AsyncRuntime;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::AsyncRuntimeOf;
use raft_kv_memstore_rkyv::TypeConfig;
use raft_kv_memstore_rkyv::app::start_raft_app;
use raft_kv_memstore_rkyv::typ::*;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing_subscriber::EnvFilter;

pub fn log_panic(panic: &PanicHookInfo) {
    let backtrace = format!("{:?}", Backtrace::force_capture());

    eprintln!("{}", panic);

    if let Some(location) = panic.location() {
        tracing::error!(
            message = %panic,
            backtrace = %backtrace,
            panic.file = location.file(),
            panic.line = location.line(),
            panic.column = location.column(),
        );
        eprintln!("{}:{}:{}", location.file(), location.line(), location.column());
    } else {
        tracing::error!(message = %panic, backtrace = %backtrace);
    }

    eprintln!("{}", backtrace);
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
enum WireRequest {
    Vote(VoteRequest),
    AppendEntries(AppendEntriesRequest),
    FullSnapshot { vote: Vote, snapshot: WireSnapshot },
    TransferLeader(openraft::raft::TransferLeaderRequest<TypeConfig>),
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct WireSnapshot {
    meta: SnapshotMeta,
    data: Vec<u8>,
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
enum WireResponse {
    Vote(Result<VoteResponse, String>),
    AppendEntries(Result<AppendEntriesResponse, String>),
    FullSnapshot(Result<SnapshotResponse, String>),
    TransferLeader(Result<(), String>),
}

/// Set up a 3-node cluster transport and verify socket-based Raft RPCs.
#[test]
fn test_cluster() {
    TypeConfig::run(test_cluster_inner()).unwrap();
}

async fn test_cluster_inner() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|panic| {
        log_panic(panic);
    }));

    let _ = tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true)
        .with_ansi(false)
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    // Node 1 spawn
    let _h1 = thread::spawn(|| {
        let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
        let x = rt.block_on(start_raft_app(1, get_addr(1)));
        println!("raft app(1) exit result: {:?}", x);
    });

    // Node 2 spawn
    let _h2 = thread::spawn(|| {
        let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
        let x = rt.block_on(start_raft_app(2, get_addr(2)));
        println!("raft app(2) exit result: {:?}", x);
    });

    // Node 3 spawn
    let _h3 = thread::spawn(|| {
        let mut rt = AsyncRuntimeOf::<TypeConfig>::new(1);
        let x = rt.block_on(start_raft_app(3, get_addr(3)));
        println!("raft app(3) exit result: {:?}", x);
    });

    wait_for_server(get_addr(1), Duration::from_secs(10)).await?;
    wait_for_server(get_addr(2), Duration::from_secs(10)).await?;
    wait_for_server(get_addr(3), Duration::from_secs(10)).await?;

    // Verify vote RPC on each node over the new framed TCP transport.
    for node_id in [1_u64, 2, 3] {
        let addr = get_addr(node_id);
        let req = VoteRequest::new(Vote::new(10, 1), None);
        let resp = send_vote_once(&addr, req.clone()).await?;

        assert!(resp.vote >= req.vote, "node {node_id} returned stale vote");
        assert!(resp.vote_granted, "node {node_id} should grant initial vote");
    }

    // Reuse a single TCP stream to verify multiple RPC frames on one socket.
    let mut stream = TcpStream::connect(get_addr(2)).await?;

    let req1 = VoteRequest::new(Vote::new(20, 1), None);
    write_frame(&mut stream, &WireRequest::Vote(req1.clone())).await?;

    let resp1: WireResponse = read_frame(&mut stream).await?;
    let vote_resp1 = match resp1 {
        WireResponse::Vote(Ok(v)) => v,
        WireResponse::Vote(Err(e)) => anyhow::bail!("vote request 1 failed: {e}"),
        other => anyhow::bail!("unexpected response to vote request 1: {other:?}"),
    };
    assert!(vote_resp1.vote >= req1.vote);
    assert!(vote_resp1.vote_granted);

    let req2 = VoteRequest::new(Vote::new(21, 3), None);
    write_frame(&mut stream, &WireRequest::Vote(req2.clone())).await?;

    let resp2: WireResponse = read_frame(&mut stream).await?;
    let vote_resp2 = match resp2 {
        WireResponse::Vote(Ok(v)) => v,
        WireResponse::Vote(Err(e)) => anyhow::bail!("vote request 2 failed: {e}"),
        other => anyhow::bail!("unexpected response to vote request 2: {other:?}"),
    };
    assert!(vote_resp2.vote >= req2.vote);

    // AppendEntries must carry a committed leader vote.
    let append_vote = Vote {
        committed: true,
        ..vote_resp2.vote
    };
    let append_req = AppendEntriesRequest {
        vote: append_vote,
        prev_log_id: None,
        entries: vec![],
        leader_commit: None,
    };

    write_frame(&mut stream, &WireRequest::AppendEntries(append_req)).await?;
    let append_resp: WireResponse = read_frame(&mut stream).await?;

    match append_resp {
        WireResponse::AppendEntries(Ok(resp)) => {
            assert!(matches!(
                resp,
                AppendEntriesResponse::Success | AppendEntriesResponse::PartialSuccess(_)
            ));
        }
        WireResponse::AppendEntries(Err(e)) => anyhow::bail!("append entries failed: {e}"),
        other => anyhow::bail!("unexpected response to append entries: {other:?}"),
    }

    Ok(())
}

async fn send_vote_once(addr: &str, req: VoteRequest) -> anyhow::Result<VoteResponse> {
    let mut stream = TcpStream::connect(addr).await.with_context(|| format!("connect to {addr}"))?;

    write_frame(&mut stream, &WireRequest::Vote(req)).await?;

    let resp: WireResponse = read_frame(&mut stream).await?;
    match resp {
        WireResponse::Vote(Ok(v)) => Ok(v),
        WireResponse::Vote(Err(e)) => anyhow::bail!("vote RPC returned error: {e}"),
        other => anyhow::bail!("unexpected vote RPC response: {other:?}"),
    }
}

async fn wait_for_server(addr: String, timeout: Duration) -> anyhow::Result<()> {
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        match TcpStream::connect(&addr).await {
            Ok(stream) => {
                drop(stream);
                return Ok(());
            }
            Err(_) => {
                TypeConfig::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    anyhow::bail!("timed out waiting for server at {addr}")
}

async fn write_frame<T>(stream: &mut TcpStream, value: &T) -> anyhow::Result<()>
where
    T: rkyv::Archive,
    T: for<'any> rkyv::Serialize<
            rkyv::api::high::HighSerializer<
                rkyv::util::AlignedVec,
                rkyv::ser::allocator::ArenaHandle<'any>,
                rkyv::rancor::Error,
            >,
        >,
{
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(value)?;
    let len = u32::try_from(bytes.len()).context("frame too large")?;

    stream.write_u32(len).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<T>(stream: &mut TcpStream) -> anyhow::Result<T>
where
    T: rkyv::Archive,
    T::Archived: for<'arch> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'arch, rkyv::rancor::Error>>
        + rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
{
    let len = stream.read_u32().await?;
    let mut buf = vec![0_u8; len as usize];
    stream.read_exact(&mut buf).await?;

    Ok(rkyv::from_bytes::<T, rkyv::rancor::Error>(&buf)?)
}

fn get_addr(node_id: u64) -> String {
    match node_id {
        1 => "127.0.0.1:21001".to_string(),
        2 => "127.0.0.1:21002".to_string(),
        3 => "127.0.0.1:21003".to_string(),
        _ => unreachable!("node_id must be 1, 2, or 3"),
    }
}
