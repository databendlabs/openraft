use std::sync::Arc;

use crate::app::App;
use crate::typ::*;

pub async fn read(app: Arc<App>, key: String) -> Result<types_kv::Response, Infallible> {
    let value = app.data.get(&key).await;

    Ok(types_kv::Response { value })
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<types_kv::Response, LinearizableReadError> {
    app.ensure_linearizable().await?;
    let value = app.data.get(&key).await;

    Ok(types_kv::Response { value })
}
