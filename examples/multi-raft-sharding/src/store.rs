//! State machine storage with TiKV-style Split support.
//!
//! This module implements a state machine that supports:
//! - Normal key-value operations (Set, Delete)
//! - **Split operation**: Atomically splits a shard into two shards
//!
//! ## Split Mechanism
//!
//! The Split operation is implemented as a special Raft log entry. When the Split log
//! is applied to the state machine:
//!
//! 1. All keys > split_point are extracted from the current state machine
//! 2. These keys are packaged as the "initial state" for the new shard
//! 3. The extracted keys are removed from the current state machine
//! 4. The response contains the initial state for bootstrapping the new shard
//!
//! ```text
//! Before Split (shard_a):
//! ┌─────────────────────────────────────┐
//! │ user:1=Alice  user:50=Bob           │
//! │ user:101=Charlie  user:150=Diana    │
//! └─────────────────────────────────────┘
//!
//! Split at user_id=100
//!
//! After Split:
//! ┌─────────────────────────────────────┐
//! │ shard_a: user:1=Alice  user:50=Bob  │  (keys <= 100)
//! └─────────────────────────────────────┘
//! ┌─────────────────────────────────────┐
//! │ shard_b: user:101=Charlie           │  (keys > 100, new shard)
//! │          user:150=Diana             │
//! └─────────────────────────────────────┘
//! ```
//!
//! ## Why This Design?
//!
//! By making Split a Raft log entry:
//! - **Atomicity**: All replicas execute split at the same log index
//! - **No Locks**: No distributed coordination needed during split
//! - **Consistency**: Split boundary is deterministic and identical on all replicas

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Debug;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;

use futures::Stream;
use futures::TryStreamExt;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use serde::Deserialize;
use serde::Serialize;

use crate::typ::*;
use crate::ShardId;
use crate::TypeConfig;

pub type LogStore = mem_log::LogStore<TypeConfig>;

// =============================================================================
// Request Types
// =============================================================================

/// These requests are proposed to Raft and applied to the state machine when committed.
/// The key design here is that `Split` is just another request type, making it atomic.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    Set {
        key: String,
        value: String,
    },

    Delete {
        key: String,
    },

    /// Split the shard at the given user_id boundary.
    ///
    /// This is the TiKV-style split operation:
    /// - All data with user_id > split_at will be moved to the new shard
    /// - The split is atomic and consistent across all replicas
    /// - After split, the new shard can be started on any node
    ///
    /// # Arguments
    /// * `split_at` - The split boundary (exclusive). Keys with user_id > split_at go to new shard.
    /// * `new_shard_id` - The ID for the new shard being created.
    ///
    /// # Example
    /// ```ignore
    /// // Split shard_a at user_id=100, creating shard_b for user_id > 100
    /// Request::Split {
    ///     split_at: 100,
    ///     new_shard_id: "shard_b".to_string(),
    /// }
    /// ```
    Split {
        /// The split point. Keys with user_id > split_at go to the new shard.
        split_at: u64,
        /// The ID for the new shard.
        new_shard_id: ShardId,
    },
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Request::Set { key, value } => write!(f, "Set({} = {})", key, value),
            Request::Delete { key } => write!(f, "Delete({})", key),
            Request::Split { split_at, new_shard_id } => {
                write!(f, "Split(at={}, new_shard={})", split_at, new_shard_id)
            }
        }
    }
}

impl Request {
    pub fn set_user(user_id: u64, value: impl ToString) -> Self {
        Self::Set {
            key: format!("user:{}", user_id),
            value: value.to_string(),
        }
    }

    /// Create a Split request.
    ///
    /// # Arguments
    /// * `split_at` - User IDs greater than this value go to the new shard
    /// * `new_shard_id` - The ID for the new shard
    pub fn split(split_at: u64, new_shard_id: impl ToString) -> Self {
        Self::Split {
            split_at,
            new_shard_id: new_shard_id.to_string(),
        }
    }
}

// =============================================================================
// Response Types
// =============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Response {
    Ok {
        previous_value: Option<String>,
    },

    /// Response for Split operation.
    /// Contains all the data needed to bootstrap the new shard.
    SplitComplete {
        /// The ID of the new shard
        new_shard_id: ShardId,
        /// The data that was split off (to be used as initial state for new shard)
        split_data: BTreeMap<String, String>,
        /// Number of keys that were split off
        key_count: usize,
    },
}

// =============================================================================
// State Machine Data
// =============================================================================

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachineData {
    pub last_applied: Option<LogId>,

    pub last_membership: StoredMembership,

    /// The key-value data store.
    ///
    /// Keys are in the format "user:{user_id}".
    /// This is a BTreeMap for efficient range operations during split.
    pub data: BTreeMap<String, String>,
}

impl StateMachineData {
    /// Extract the user_id from a key string.
    /// Returns None if the key doesn't match the expected format "user:{id}".
    pub fn parse_user_id(key: &str) -> Option<u64> {
        key.strip_prefix("user:").and_then(|s| s.parse().ok())
    }

    /// Get all keys with user_id greater than the given value.
    /// This is used during split to extract data for the new shard.
    pub fn keys_greater_than(&self, user_id: u64) -> BTreeMap<String, String> {
        self.data
            .iter()
            .filter(|(k, _)| Self::parse_user_id(k).map(|id| id > user_id).unwrap_or(false))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Remove all keys with user_id greater than the given value.
    /// This is used during split to clean up the original shard after data extraction.
    pub fn remove_keys_greater_than(&mut self, user_id: u64) {
        self.data.retain(|k, _| {
            Self::parse_user_id(k).map(|id| id <= user_id).unwrap_or(true) // Keep non-user keys
        });
    }
}

// =============================================================================
// Stored Snapshot
// =============================================================================

#[derive(Debug)]
pub struct StoredSnapshot {
    pub meta: SnapshotMeta,
    pub data: SnapshotData,
}

// =============================================================================
// State Machine Store
// =============================================================================

#[derive(Debug, Default)]
pub struct StateMachineStore {
    pub state_machine: tokio::sync::Mutex<StateMachineData>,

    /// Counter for generating unique snapshot IDs.
    snapshot_idx: Mutex<u64>,

    /// The current snapshot (if any).
    current_snapshot: Mutex<Option<StoredSnapshot>>,
}

impl StateMachineStore {
    pub fn with_initial_data(data: BTreeMap<String, String>) -> Self {
        Self {
            state_machine: tokio::sync::Mutex::new(StateMachineData {
                last_applied: None,
                last_membership: StoredMembership::default(),
                data,
            }),
            snapshot_idx: Mutex::new(0),
            current_snapshot: Mutex::new(None),
        }
    }
}

// =============================================================================
// Snapshot Builder Implementation
// =============================================================================

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot, io::Error> {
        let data;
        let last_applied_log;
        let last_membership;

        {
            let state_machine = self.state_machine.lock().await.clone();
            last_applied_log = state_machine.last_applied;
            last_membership = state_machine.last_membership.clone();
            data = state_machine;
        }

        let snapshot_idx = {
            let mut l = self.snapshot_idx.lock().unwrap();
            *l += 1;
            *l
        };

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.committed_leader_id(), last.index(), snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        {
            let mut current_snapshot = self.current_snapshot.lock().unwrap();
            *current_snapshot = Some(snapshot);
        }

        Ok(Snapshot { meta, snapshot: data })
    }
}

// =============================================================================
// State Machine Implementation
// =============================================================================

impl RaftStateMachine<TypeConfig> for Arc<StateMachineStore> {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), io::Error> {
        let state_machine = self.state_machine.lock().await;
        Ok((state_machine.last_applied, state_machine.last_membership.clone()))
    }

    /// Apply committed log entries to the state machine.
    ///
    /// This is where the magic happens for Split operations:
    /// - When a Split log is applied, we atomically extract data for the new shard
    /// - The response contains the extracted data for bootstrapping the new shard
    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        let mut sm = self.state_machine.lock().await;

        while let Some((entry, responder)) = entries.try_next().await? {
            tracing::debug!(%entry.log_id, "applying entry to state machine");

            sm.last_applied = Some(entry.log_id);

            let response = match entry.payload {
                EntryPayload::Blank => Response::Ok { previous_value: None },

                EntryPayload::Normal(ref req) => {
                    match req {
                        Request::Set { key, value } => {
                            // Normal set operation
                            let previous = sm.data.insert(key.clone(), value.clone());
                            tracing::debug!(key = %key, value = %value, "set key");
                            Response::Ok {
                                previous_value: previous,
                            }
                        }

                        Request::Delete { key } => {
                            // Normal delete operation
                            let previous = sm.data.remove(key);
                            tracing::debug!(key = %key, "delete key");
                            Response::Ok {
                                previous_value: previous,
                            }
                        }

                        Request::Split { split_at, new_shard_id } => {
                            // ============================================================
                            // SPLIT OPERATION - The Key Feature of This Example
                            // ============================================================
                            //
                            // This is where the TiKV-style split happens:
                            // 1. Extract all keys with user_id > split_at
                            // 2. Remove those keys from the current state machine
                            // 3. Return the extracted data in the response
                            //
                            // The response will be used to bootstrap the new shard.

                            tracing::info!(
                                split_at = %split_at,
                                new_shard_id = %new_shard_id,
                                "executing split operation"
                            );

                            // Step 1: Extract data for the new shard
                            let split_data = sm.keys_greater_than(*split_at);
                            let key_count = split_data.len();

                            tracing::info!(
                                key_count = %key_count,
                                "extracted {} keys for new shard",
                                key_count
                            );

                            // Step 2: Remove extracted keys from current shard
                            sm.remove_keys_greater_than(*split_at);

                            tracing::info!(
                                remaining_keys = %sm.data.len(),
                                "current shard now has {} keys",
                                sm.data.len()
                            );

                            // Step 3: Return the split data in the response
                            Response::SplitComplete {
                                new_shard_id: new_shard_id.clone(),
                                split_data,
                                key_count,
                            }
                        }
                    }
                }

                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    Response::Ok { previous_value: None }
                }
            };

            if let Some(responder) = responder {
                responder.send(response);
            }
        }
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn begin_receiving_snapshot(&mut self) -> Result<SnapshotData, io::Error> {
        Ok(Default::default())
    }

    #[tracing::instrument(level = "trace", skip(self, snapshot))]
    async fn install_snapshot(&mut self, meta: &SnapshotMeta, snapshot: SnapshotData) -> Result<(), io::Error> {
        tracing::info!(
            snapshot_id = %meta.snapshot_id,
            keys = %snapshot.data.len(),
            "installing snapshot"
        );

        let new_snapshot = StoredSnapshot {
            meta: meta.clone(),
            data: snapshot,
        };

        // Update the state machine
        {
            let updated_state_machine: StateMachineData = new_snapshot.data.clone();
            let mut state_machine = self.state_machine.lock().await;
            *state_machine = updated_state_machine;
        }

        // Update current snapshot
        let mut current_snapshot = self.current_snapshot.lock().unwrap();
        *current_snapshot = Some(new_snapshot);
        Ok(())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, io::Error> {
        match &*self.current_snapshot.lock().unwrap() {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot {
                    meta: snapshot.meta.clone(),
                    snapshot: data,
                }))
            }
            None => Ok(None),
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_id() {
        assert_eq!(StateMachineData::parse_user_id("user:123"), Some(123));
        assert_eq!(StateMachineData::parse_user_id("user:0"), Some(0));
        assert_eq!(StateMachineData::parse_user_id("other:123"), None);
        assert_eq!(StateMachineData::parse_user_id("user:abc"), None);
    }

    #[test]
    fn test_keys_greater_than() {
        let mut data = BTreeMap::new();
        data.insert("user:50".to_string(), "Alice".to_string());
        data.insert("user:100".to_string(), "Bob".to_string());
        data.insert("user:101".to_string(), "Charlie".to_string());
        data.insert("user:200".to_string(), "Diana".to_string());

        let sm = StateMachineData {
            data,
            ..Default::default()
        };

        let split_data = sm.keys_greater_than(100);
        assert_eq!(split_data.len(), 2);
        assert!(split_data.contains_key("user:101"));
        assert!(split_data.contains_key("user:200"));
        assert!(!split_data.contains_key("user:100"));
    }

    #[test]
    fn test_remove_keys_greater_than() {
        let mut data = BTreeMap::new();
        data.insert("user:50".to_string(), "Alice".to_string());
        data.insert("user:100".to_string(), "Bob".to_string());
        data.insert("user:101".to_string(), "Charlie".to_string());
        data.insert("user:200".to_string(), "Diana".to_string());

        let mut sm = StateMachineData {
            data,
            ..Default::default()
        };
        sm.remove_keys_greater_than(100);

        assert_eq!(sm.data.len(), 2);
        assert!(sm.data.contains_key("user:50"));
        assert!(sm.data.contains_key("user:100"));
        assert!(!sm.data.contains_key("user:101"));
    }
}
