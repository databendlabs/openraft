use std::sync::Arc;

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
