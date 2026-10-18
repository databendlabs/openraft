use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use openraft::RaftLogReader;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::decompose::DecomposeResult;
use serde::Serialize;

use crate::app::App;
use crate::typ::*;

pub async fn read(app: Arc<App>, key: String) -> Result<types_kv::Response, Infallible> {
    let state = app.data.state.lock().await;
    let value = state.kvs.get(&key);

    Ok(types_kv::Response { value: value.cloned() })
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<types_kv::Response, LinearizableReadError> {
    app.ensure_linearizable().await?;

    let state = app.data.state.lock().await;
    let value = state.kvs.get(&key);

    Ok(types_kv::Response { value: value.cloned() })
}

/// Ask this node, if it leads, to hand leadership to `to`. A non-leader ignores the request, so
/// the caller observes the outcome in the metrics.
pub async fn transfer_leader(app: Arc<App>, to: crate::NodeId) -> Result<(), Fatal> {
    app.raft.trigger().transfer_leader(to).await
}

/// Append one explicit membership, allowing a test to separate joint and final entries.
pub(crate) async fn append_membership(
    app: Arc<App>,
    configs: Vec<BTreeSet<crate::NodeId>>,
) -> Result<ClientWriteResponse, ClientWriteError> {
    let metrics = app.raft.metrics().borrow_watched().clone();
    let nodes = metrics
        .membership_config
        .membership()
        .nodes()
        .map(|(id, node)| (id.clone(), node.clone()))
        .collect::<BTreeMap<_, _>>();
    let membership = openraft::Membership::new(configs, nodes)
        .map_err(|error| ClientWriteError::ChangeMembershipError(error.into()))?;
    app.raft.append_membership(membership, openraft::EntryPayload::Blank, []).await.decompose().unwrap()
}

/// Read a list after a ReadIndex check; a list nothing was appended to reads as empty.
pub async fn linearizable_read_list(app: Arc<App>, key: String) -> Result<Vec<String>, LinearizableReadError> {
    app.ensure_linearizable().await?;

    let state = app.data.state.lock().await;
    Ok(state.lists.get(&key).cloned().unwrap_or_default())
}

/// One replica's applied state, for Jepsen to compare replicas once they have applied the same
/// log.
#[derive(Debug, Serialize)]
pub struct Consistency {
    pub last_applied: Option<LogId>,

    /// Digest of every entry applied so far, carried through snapshots, in hex.
    pub applied_chain: String,

    /// Digest of the key-value map and the membership, in hex.
    pub state_digest: String,

    /// Registers and lists together.
    pub key_count: usize,

    /// Log ids of the entries still in the log store, up to `last_applied`.
    pub log_ids: Vec<LogId>,
}

/// Reports `last_applied` and both digests from one instant, then the retained log ids.
pub async fn consistency(app: Arc<App>) -> Result<Consistency, String> {
    let (last_applied, applied_chain, state_digest, key_count) = {
        let state = app.data.state.lock().await;
        let state_digest = state.state_digest().map_err(|e| e.to_string())?;
        (
            state.last_applied.clone(),
            state.applied_chain,
            state_digest,
            state.kvs.len() + state.lists.len(),
        )
    };

    // Entries up to `last_applied` are committed, so they are never truncated, only purged.
    let log_ids = match &last_applied {
        None => Vec::new(),
        Some(applied) => {
            let mut reader = app.data.log_store.clone();
            let entries = reader.try_get_log_entries(..=applied.index()).await.map_err(|e| e.to_string())?;
            entries.into_iter().map(|entry| entry.log_id).collect()
        }
    };

    Ok(Consistency {
        last_applied,
        applied_chain: format!("{applied_chain:016x}"),
        state_digest: format!("{state_digest:016x}"),
        key_count,
        log_ids,
    })
}
