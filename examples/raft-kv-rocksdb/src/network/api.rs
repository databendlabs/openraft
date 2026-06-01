use std::sync::Arc;

use crate::app::App;
use crate::typ::*;

pub async fn read(app: Arc<App>, key: String) -> Result<String, Infallible> {
    let kvs = app.data.lock().await;
    let value = kvs.get(&key);

    Ok(value.cloned().unwrap_or_default())
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<String, LinearizableReadError> {
    app.ensure_linearizable().await?;

    let kvs = app.data.lock().await;
    let value = kvs.get(&key);

    Ok(value.cloned().unwrap_or_default())
}
