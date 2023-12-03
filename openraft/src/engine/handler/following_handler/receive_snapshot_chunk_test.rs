use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::sm;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::error::InstallSnapshotError;
use crate::error::SnapshotMismatch;
use crate::raft::InstallSnapshotRequest;
use crate::raft_state::StreamingState;
use crate::testing::log_id;
use crate::Membership;
use crate::SnapshotMeta;
use crate::SnapshotSegmentId;
use crate::StoredMembership;
use crate::TokioInstant;
use crate::Vote;

fn m1234() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2,3,4}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.state.vote.update(TokioInstant::now(), Vote::new_committed(2, 1));
    eng.state.server_state = eng.calc_server_state();

    eng
}

fn make_meta() -> SnapshotMeta<u64, ()> {
    SnapshotMeta {
        last_log_id: Some(log_id(2, 1, 2)),
        last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    }
}

fn make_req(offset: u64) -> InstallSnapshotRequest<UTConfig> {
    InstallSnapshotRequest {
        vote: Vote::new_committed(2, 1),
        meta: make_meta(),
        offset,
        data: vec![],
        done: false,
    }
}

#[test]
fn test_receive_snapshot_chunk_new_chunk() -> anyhow::Result<()> {
    let mut eng = eng();
    assert!(eng.state.snapshot_streaming.is_none());

    eng.following_handler().receive_snapshot_chunk(make_req(0))?;

    assert_eq!(
        Some(StreamingState {
            offset: 0,
            snapshot_id: "1-2-3-4".to_string(),
        }),
        eng.state.snapshot_streaming
    );
    assert_eq!(
        vec![Command::from(sm::Command::receive(make_req(0)).with_seq(1))],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_receive_snapshot_chunk_continue_receive_chunk() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.state.snapshot_streaming = Some(StreamingState {
        offset: 0,
        snapshot_id: "1-2-3-4".to_string(),
    });

    eng.following_handler().receive_snapshot_chunk(make_req(2))?;

    assert_eq!(
        Some(StreamingState {
            offset: 2,
            snapshot_id: "1-2-3-4".to_string(),
        }),
        eng.state.snapshot_streaming
    );
    assert_eq!(
        vec![Command::from(sm::Command::receive(make_req(2)).with_seq(1))],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_receive_snapshot_chunk_diff_id_offset_0() -> anyhow::Result<()> {
    // When receiving a chunk with different snapshot id and offset 0, starts a new snapshot streaming.
    let mut eng = eng();

    eng.state.snapshot_streaming = Some(StreamingState {
        offset: 2,
        snapshot_id: "1-2-3-100".to_string(),
    });

    eng.following_handler().receive_snapshot_chunk(make_req(0))?;

    assert_eq!(
        Some(StreamingState {
            offset: 0,
            snapshot_id: "1-2-3-4".to_string(),
        }),
        eng.state.snapshot_streaming
    );
    assert_eq!(
        vec![Command::from(sm::Command::receive(make_req(0)).with_seq(1))],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_receive_snapshot_chunk_diff_id_offset_gt_0() -> anyhow::Result<()> {
    // When receiving a chunk with different snapshot id and offset that is greater than 0, return an
    // error.
    let mut eng = eng();

    eng.state.snapshot_streaming = Some(StreamingState {
        offset: 2,
        snapshot_id: "1-2-3-100".to_string(),
    });

    let res = eng.following_handler().receive_snapshot_chunk(make_req(3));

    assert_eq!(
        Err(InstallSnapshotError::from(SnapshotMismatch {
            expect: SnapshotSegmentId {
                id: "1-2-3-4".to_string(),
                offset: 0
            },
            got: SnapshotSegmentId {
                id: "1-2-3-4".to_string(),
                offset: 3
            },
        })),
        res
    );

    assert_eq!(
        Some(StreamingState {
            offset: 2,
            snapshot_id: "1-2-3-100".to_string(),
        }),
        eng.state.snapshot_streaming,
        "streaming state not changed"
    );
    assert_eq!(true, eng.output.take_commands().is_empty());

    Ok(())
}
