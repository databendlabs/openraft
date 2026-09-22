use std::fs;
use std::io;

use bytes::Bytes;
use dir_transfer::DirFrame;
use futures::stream;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use raft_kv_rocksdb::TypeConfig;
use raft_kv_rocksdb::typ::AppendEntriesRequest;
use raft_kv_rocksdb::typ::RaftError;
use raft_kv_rocksdb::typ::SnapshotResponse;
use raft_kv_rocksdb::typ::Vote;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::node::Node;
use crate::node::POLL_INTERVAL;
use crate::node::TIMEOUT;
use crate::snapshot_transfer::SnapshotTransfer;

const CHUNK_SIZE: usize = 4096;
const VALUE_SIZE: usize = 128 * 1024;
const FRAGMENT_SIZE: usize = 97;

#[test]
fn test_interrupted_snapshot_preserves_state_and_retries() -> anyhow::Result<()> {
    const PORT_BASE: u16 = 11000;
    TypeConfig::run(async {
        let leader = Node::new(1, PORT_BASE).await?;
        let learner = Node::new(2, PORT_BASE).await?;
        leader.initialize().await?;

        tracing::info!("Install a baseline before interrupting a replacement checkpoint");
        let baseline = {
            let written = leader.write("key", "old").await?;
            let snapshot = leader.build_snapshot().await?;
            let transfer = SnapshotTransfer::new(&leader, &snapshot, CHUNK_SIZE).await?;
            let response = transfer.send(&learner).await?;
            let expected_response = SnapshotResponse::new(transfer.vote);
            assert_eq!(expected_response, response);
            written.data
        };
        let state_before = learner.applied_state().await?;
        let current_before = learner.current()?;
        let generations_before = learner.generations()?;

        tracing::info!("Capture a replacement containing a file larger than one transfer chunk");
        let (expected, transfer) = {
            let value = large_value();
            let written = leader.write("key", &value).await?;
            let snapshot = leader.build_snapshot().await?;
            let transfer = SnapshotTransfer::new(&leader, &snapshot, CHUNK_SIZE).await?;
            (written.data, transfer)
        };

        tracing::info!(dir = %learner.dir().display(), "Disconnect after the receiver writes part of a file");
        {
            let messages = transfer.messages()?;
            let mut prefix = messages[0].clone();
            for (index, frame) in transfer.frames.iter().enumerate() {
                prefix.extend_from_slice(&messages[index + 1]);
                let DirFrame::Chunk { data } = frame else {
                    continue;
                };
                if data.len() == CHUNK_SIZE {
                    break;
                }
            }
            let complete = transfer.body()?;
            let interrupted_mid_transfer = prefix.len() < complete.len();
            assert!(interrupted_mid_transfer);
            let header = format!(
                "POST /snapshot HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                learner.raft_addr(),
                complete.len()
            );
            let mut connection = TcpStream::connect(learner.raft_addr()).await?;
            connection.write_all(header.as_bytes()).await?;
            connection.write_all(&prefix).await?;
            let receiving = async {
                loop {
                    let generations = learner.generations()?;
                    for path in generations.difference(&generations_before) {
                        for entry in fs::read_dir(path)? {
                            let entry = entry?;
                            let metadata = entry.metadata()?;
                            let chunk_written = metadata.is_file() && metadata.len() >= CHUNK_SIZE as u64;
                            if chunk_written {
                                return Ok::<_, anyhow::Error>(());
                            }
                        }
                    }
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            };
            let received = tokio::time::timeout(TIMEOUT, receiving).await?;
            received?;
            drop(connection);
            learner.wait_generations(&generations_before).await?;
            let actual = learner.read("key").await?;
            assert_eq!(baseline, actual);
            let actual_state = learner.applied_state().await?;
            assert_eq!(state_before, actual_state);
            let actual_current = learner.current()?;
            assert_eq!(current_before, actual_current);
        }

        tracing::info!("Retry the complete checkpoint after the interrupted directory is removed");
        {
            let response = transfer.send(&learner).await?;
            let expected_response = SnapshotResponse::new(transfer.vote);
            assert_eq!(expected_response, response);
            let actual = learner.read("key").await?;
            assert_eq!(expected, actual);
            let expected_state = leader.applied_state().await?;
            let actual_state = learner.applied_state().await?;
            assert_eq!(expected_state, actual_state);
        }

        leader.cleanup().await?;
        learner.cleanup().await?;
        Ok(())
    })
}

#[test]
fn test_large_snapshot_with_fragmented_http_body() -> anyhow::Result<()> {
    const PORT_BASE: u16 = 13000;
    TypeConfig::run(async {
        let leader = Node::new(1, PORT_BASE).await?;
        let learner = Node::new(2, PORT_BASE).await?;
        leader.initialize().await?;

        tracing::info!("Build a checkpoint whose file data requires multiple full chunks");
        let (expected, transfer) = {
            let value = large_value();
            let written = leader.write("large", &value).await?;
            let second = leader.write("second", "another value").await?;
            let snapshot = leader.build_snapshot().await?;
            let transfer = SnapshotTransfer::new(&leader, &snapshot, CHUNK_SIZE).await?;
            let mut full_chunks = 0;
            for frame in &transfer.frames {
                let DirFrame::Chunk { data } = frame else {
                    continue;
                };
                if data.len() == CHUNK_SIZE {
                    full_chunks += 1;
                }
            }
            let spans_multiple_chunks = full_chunks > 1;
            assert!(spans_multiple_chunks);
            ((written.data, second.data), transfer)
        };

        tracing::info!("Split length prefixes and payloads across HTTP chunks, combining message boundaries");
        {
            let bytes = transfer.body()?;
            let first = Bytes::copy_from_slice(&bytes[..1]);
            let second = Bytes::copy_from_slice(&bytes[1..3]);
            let mut fragments = vec![Ok::<_, io::Error>(first), Ok(second)];
            for chunk in bytes[3..].chunks(FRAGMENT_SIZE) {
                let chunk = Bytes::copy_from_slice(chunk);
                fragments.push(Ok(chunk));
            }
            let fragments = stream::iter(fragments);
            let body = reqwest::Body::wrap_stream(fragments);
            let response = learner.post_snapshot(body).await?;
            let status = response.status();
            assert_eq!(reqwest::StatusCode::OK, status);
            let result: Result<SnapshotResponse, RaftError> = response.json().await?;
            let response = result?;
            let expected_response = SnapshotResponse::new(transfer.vote);
            assert_eq!(expected_response, response);
            let actual = learner.read("large").await?;
            assert_eq!(expected.0, actual);
            let second = learner.read("second").await?;
            assert_eq!(expected.1, second);
            let expected_state = leader.applied_state().await?;
            let actual_state = learner.applied_state().await?;
            assert_eq!(expected_state, actual_state);
        }

        leader.cleanup().await?;
        learner.cleanup().await?;
        Ok(())
    })
}

#[test]
fn test_invalid_snapshot_payloads_preserve_state() -> anyhow::Result<()> {
    const PORT_BASE: u16 = 15000;
    TypeConfig::run(async {
        let leader = Node::new(1, PORT_BASE).await?;
        let learner = Node::new(2, PORT_BASE).await?;
        leader.initialize().await?;

        tracing::info!("Install state that every malformed replacement must preserve");
        let (expected, transfer) = {
            let written = leader.write("key", "preserved").await?;
            let snapshot = leader.build_snapshot().await?;
            let transfer = SnapshotTransfer::new(&leader, &snapshot, CHUNK_SIZE).await?;
            transfer.send(&learner).await?;
            (written.data, transfer)
        };
        let state_before = learner.applied_state().await?;
        let current_before = learner.current()?;
        let generations_before = learner.generations()?;

        tracing::info!("Corrupt a file payload while retaining its original checksum");
        let corrupt_body = {
            let mut corrupt = transfer.clone();
            let mut changed = false;
            for frame in &mut corrupt.frames {
                let DirFrame::Chunk { data } = frame else {
                    continue;
                };
                data[0] ^= 1;
                changed = true;
                break;
            }
            assert!(changed);
            let messages = corrupt.messages()?;
            let mut bytes = Vec::new();
            for message in &messages[1..] {
                bytes.extend_from_slice(message);
            }
            bytes
        };
        let cases = [
            ("truncated length prefix", vec![0, 0]),
            ("truncated payload", vec![0, 0, 0, 4, 0]),
            ("invalid MessagePack", vec![0, 0, 0, 1, 0xc1]),
            ("oversized message", u32::MAX.to_be_bytes().to_vec()),
            ("corrupt file contents", corrupt_body),
        ];
        let messages = transfer.messages()?;

        for (case, bytes) in cases {
            tracing::info!(case, "Reject malformed HTTP input and discard its receiving directory");
            {
                let mut body = messages[0].clone();
                body.extend(bytes);
                let body = reqwest::Body::from(body);
                let response = learner.post_snapshot(body).await?;
                let status = response.status();
                assert_eq!(reqwest::StatusCode::BAD_REQUEST, status, "{case}");
                learner.wait_generations(&generations_before).await?;
                let actual = learner.read("key").await?;
                assert_eq!(expected, actual, "{case}");
                let actual_state = learner.applied_state().await?;
                assert_eq!(state_before, actual_state, "{case}");
                let actual_current = learner.current()?;
                assert_eq!(current_before, actual_current, "{case}");
            }
        }

        leader.cleanup().await?;
        learner.cleanup().await?;
        Ok(())
    })
}

#[test]
fn test_stale_and_duplicate_snapshots_are_discarded() -> anyhow::Result<()> {
    const PORT_BASE: u16 = 17000;
    TypeConfig::run(async {
        let leader = Node::new(1, PORT_BASE).await?;
        let learner = Node::new(2, PORT_BASE).await?;
        leader.initialize().await?;

        tracing::info!("Capture an older checkpoint and install its newer replacement");
        let (older, current, expected) = {
            leader.write("key", "older").await?;
            let snapshot = leader.build_snapshot().await?;
            let older = SnapshotTransfer::new(&leader, &snapshot, CHUNK_SIZE).await?;
            let written = leader.write("key", "current").await?;
            let snapshot = leader.build_snapshot().await?;
            let current = SnapshotTransfer::new(&leader, &snapshot, CHUNK_SIZE).await?;
            current.send(&learner).await?;
            (older, current, written.data)
        };
        let state_before = learner.applied_state().await?;
        let current_before = learner.current()?;
        let generations_before = learner.generations()?;

        for transfer in [&older, &current] {
            tracing::info!(snapshot = ?transfer.meta.last_log_id, "Discard an older or duplicate checkpoint");
            {
                let response = transfer.send(&learner).await?;
                let expected_response = SnapshotResponse::new(current.vote);
                assert_eq!(expected_response, response);
                learner.wait_generations(&generations_before).await?;
                let actual = learner.read("key").await?;
                assert_eq!(expected, actual);
                let actual_state = learner.applied_state().await?;
                assert_eq!(state_before, actual_state);
                let actual_current = learner.current()?;
                assert_eq!(current_before, actual_current);
            }
        }

        tracing::info!("Reject a newer checkpoint when the receiver has already observed a higher vote");
        {
            leader.write("key", "rejected lower vote").await?;
            let snapshot = leader.build_snapshot().await?;
            let transfer = SnapshotTransfer::new(&leader, &snapshot, CHUNK_SIZE).await?;
            let higher_vote = Vote::new_committed(2, 1);
            let request = AppendEntriesRequest {
                vote: higher_vote,
                prev_log_id: None,
                entries: vec![],
                leader_commit: None,
            };
            learner.raft().append_entries(request).await?;
            let raft = learner.raft();
            let actual_vote = raft.with_raft_state(|state| *state.vote_ref()).await?;
            assert_eq!(higher_vote, actual_vote);
            let response = transfer.send(&learner).await?;
            let expected_response = SnapshotResponse::new(higher_vote);
            assert_eq!(expected_response, response);
            learner.wait_generations(&generations_before).await?;
            let actual = learner.read("key").await?;
            assert_eq!(expected, actual);
            let actual_state = learner.applied_state().await?;
            assert_eq!(state_before, actual_state);
            let actual_current = learner.current()?;
            assert_eq!(current_before, actual_current);
            let metrics = learner.raft().metrics();
            let installed = metrics.borrow_watched().snapshot;
            assert_eq!(current.meta.last_log_id, installed);
        }

        leader.cleanup().await?;
        learner.cleanup().await?;
        Ok(())
    })
}

fn large_value() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut state = 1u64;
    let mut value = String::with_capacity(VALUE_SIZE);
    for _ in 0..VALUE_SIZE {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let index = state as usize % ALPHABET.len();
        let byte = ALPHABET[index];
        let character = char::from(byte);
        value.push(character);
    }
    value
}
