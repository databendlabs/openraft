use std::sync::Arc;

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

/// Raft owns the state machine, so a read runs on the state-machine worker.
async fn get(app: &App, key: String) -> Option<types_kv::VersionedValue> {
    let read = app.raft.with_state_machine(move |sm: &mut StateMachineStore| Box::pin(async move { sm.get(&key) }));
    let value = read.await.expect("raft is running");
    value.expect("the state machine database is readable")
}
