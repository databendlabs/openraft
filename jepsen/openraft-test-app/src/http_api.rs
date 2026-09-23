use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use openraft::async_runtime::WatchReceiver;
use openraft::errors::decompose::DecomposeResult;

use crate::app::App;
use crate::typ::*;

pub async fn read(app: Arc<App>, key: String) -> Result<types_kv::Response, Infallible> {
    let kvs = app.data.lock().await;
    let value = kvs.get(&key);

    Ok(types_kv::Response { value: value.cloned() })
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<types_kv::Response, LinearizableReadError> {
    app.ensure_linearizable().await?;

    let kvs = app.data.lock().await;
    let value = kvs.get(&key);

    Ok(types_kv::Response { value: value.cloned() })
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
