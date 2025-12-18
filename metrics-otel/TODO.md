# Future Metrics TODO

Metrics that could be added to `MetricsRecorder` when OpenRaft provides the necessary instrumentation points.

## Latency / Timing

- `record_election_duration(duration_ms)` - Time from election start to becoming leader
- `record_replication_latency(peer_id, duration_ms)` - Replication round-trip time per peer
- `record_commit_latency(duration_ms)` - Time from log append to commit
- `record_storage_append_latency(duration_ms)` - Storage append duration
- `record_storage_read_latency(duration_ms)` - Storage read duration
- `record_snapshot_build_duration(duration_ms)` - Time to build snapshot
- `record_snapshot_apply_duration(duration_ms)` - Time to apply snapshot

## State Transitions

- `record_state_transition(from_state, to_state)` - Track state changes with labels
- `increment_election_started()` - Elections initiated
- `increment_election_won()` - Elections won
- `increment_election_lost()` - Elections lost (timeout or lost to another candidate)

## Errors

- `increment_storage_error()` - Storage operation failures
- `increment_network_error(peer_id)` - Network failures per peer
- `increment_vote_rejected()` - Vote rejections received
- `increment_append_rejected()` - AppendEntries rejections

## Replication Health

- `set_replication_lag(peer_id, lag_entries)` - Replication lag per peer
- `record_replication_rpc_rate(peer_id, rpc_count)` - RPCs per second per peer
- `record_replication_entry_rate(peer_id, entry_count)` - Entries replicated per second per peer

## Membership

- `set_membership_size(voters, learners)` - Current cluster membership size
- `increment_membership_change()` - Membership configuration changes

## Resource Usage

- `set_log_cache_size(bytes)` - In-memory log cache size
- `set_pending_proposals(count)` - Pending client proposals
