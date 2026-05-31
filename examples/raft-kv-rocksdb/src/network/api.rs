use std::sync::Arc;

use openraft::ReadPolicy;
use openraft::errors::decompose::DecomposeResult;

use crate::app::App;
use crate::typ::*;

pub async fn write(app: Arc<App>, req: types_kv::Request) -> Result<ClientWriteResponse, ClientWriteError> {
    app.raft.client_write(req).await.decompose().unwrap()
}

pub async fn read(app: Arc<App>, key: String) -> Result<String, Infallible> {
    let kvs = app.key_values.lock().await;
    let value = kvs.get(&key);

    Ok(value.cloned().unwrap_or_default())
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<String, LinearizableReadError> {
    let ret = app.raft.get_read_linearizer(ReadPolicy::ReadIndex).await.decompose().unwrap();

    match ret {
        Ok(linearizer) => {
            linearizer.await_ready(&app.raft).await.unwrap();

            let kvs = app.key_values.lock().await;
            let value = kvs.get(&key);

            Ok(value.cloned().unwrap_or_default())
        }
        Err(e) => Err(e),
    }
}
