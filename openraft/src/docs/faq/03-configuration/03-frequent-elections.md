### Frequent leader elections and timeouts

**Symptom**: Logs show repeated leader elections, or [`RaftMetrics::current_leader`][] changes frequently

**Cause**: [`Config::election_timeout_min`][] is too small for your storage or network latency.
If [`RaftLogStorage::append`][] takes longer than the election timeout, heartbeats time out and
trigger elections.

**Solution**: Increase both [`Config::election_timeout_min`][] and [`Config::election_timeout_max`][].
Ensure `heartbeat_interval < election_timeout_min / 2` and that election timeout is at least
10× your typical [`RaftLogStorage::append`][] latency.

[`RaftMetrics::current_leader`]: `crate::metrics::RaftMetrics::current_leader`
[`Config::election_timeout_min`]: `crate::config::Config::election_timeout_min`
[`Config::election_timeout_max`]: `crate::config::Config::election_timeout_max`
[`RaftLogStorage::append`]: `crate::storage::RaftLogStorage::append`
