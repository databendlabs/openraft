use crate::engine::testing::blank_ent;
use crate::engine::testing::UTCfg;
use crate::engine::Command;
use crate::engine::Respond;
use crate::progress::Inflight;
use crate::raft::VoteRequest;
use crate::raft_types::MetricsChangeFlags;
use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Entry;
use crate::Membership;
use crate::SnapshotMeta;
use crate::StoredMembership;
use crate::Vote;

#[test]
#[rustfmt::skip]
fn test_command_update_metrics_flags() -> anyhow::Result<()> {
    //
    fn t(c: Command<u64, (), Entry<UTCfg>>, repl: bool, data: bool, cluster: bool) {
        let mut flags = MetricsChangeFlags::default();

        c.update_metrics_flags(&mut flags);
        assert_eq!(repl, flags.replication, "{:?}", c);
        assert_eq!(data, flags.local_data, "{:?}", c);
        assert_eq!(cluster, flags.cluster, "{:?}", c);
    }

    t(Command::BecomeLeader, false, false, true);
    t(Command::QuitLeader, false, false, true);
    t(Command::AppendEntry { entry: blank_ent(1, 1) }, false, true, false);
    t(Command::AppendInputEntries { entries: vec![blank_ent(1, 1)] }, false, true, false);
    t(Command::AppendBlankLog { log_id: log_id(1,1) }, false, true, false);
    t(Command::ReplicateCommitted { committed: None }, false, false, false);
    t(Command::LeaderCommit { already_committed: None, upto: log_id(1,2) }, false, true, false);
    t(Command::FollowerCommit { already_committed: None, upto: log_id(1,2) }, false, true, false);
    t(Command::Replicate { target: 3, req: Inflight::None }, false, false, false);
    t(Command::UpdateMembership{ membership: EffectiveMembership::new_arc(Some(log_id(1,1)), Membership::new(vec![], ()) ) }, false, false, true);
    t(Command::RebuildReplicationStreams{ targets: vec![] }, true, false, false);
    t(Command::UpdateProgressMetrics{ target: 0, matching: log_id(1,2), }, true, false, false);
    t(Command::SaveVote{ vote: Vote::new(1,2) }, false, true, false);
    t(Command::SendVote{ vote_req: VoteRequest { vote: Vote::new(1,2), last_log_id: None } }, false, false, false);
    t(Command::PurgeLog{ upto: log_id(1, 2) }, false, true, false);
    t(Command::DeleteConflictLog{ since: log_id(1,2) }, false, true, false);
    t(Command::InstallSnapshot{ snapshot_meta: SnapshotMeta {
        last_log_id: None,
        last_membership: StoredMembership::new(Some(log_id(1,2)), Membership::new(vec![], ())),
        snapshot_id: "".to_string(),
    } }, false, true, false);
    t(Command::CancelSnapshot{ snapshot_meta: SnapshotMeta {
        last_log_id: None,
        last_membership: StoredMembership::new(Some(log_id(1,2)), Membership::new(vec![], ())),
        snapshot_id: "".to_string(),
    } }, false, false, false);
    t(Command::BuildSnapshot{} , false, true, false);
    
    let (tx, _rx) = tokio::sync::oneshot::channel();
    t(Command::Respond { resp: Respond::new(Ok(()), tx) }, false, false, false);

    Ok(())
}
