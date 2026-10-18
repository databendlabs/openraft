use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;

use openraft::async_runtime::WatchReceiver;
use openraft::errors::decompose::DecomposeResult;

use crate::StateMachineStore;
use crate::app::App;
use crate::typ::*;

pub async fn read(app: Arc<App>, key: String) -> Result<types_kv::Response, Infallible> {
    let value = get(&app, key).await;

    Ok(types_kv::Response { value })
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<types_kv::Response, LinearizableReadError> {
    app.ensure_linearizable().await?;

    let value = get(&app, key).await;

    Ok(types_kv::Response { value })
}

/// Read through the worker so snapshot installation cannot leave a stale database handle.
async fn get(app: &App, key: String) -> Option<types_kv::VersionedValue> {
    let read = app.raft.with_state_machine(move |sm: &mut StateMachineStore| Box::pin(async move { sm.get(&key) }));
    let value = read.await.expect("raft is running");
    value.expect("the state machine database is readable")
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
