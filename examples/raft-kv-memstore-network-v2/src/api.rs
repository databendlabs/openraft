use std::sync::Arc;

use crate::app::App;
use crate::typ::*;

pub async fn read(app: Arc<App>, key: String) -> Result<String, Infallible> {
    let value = app.data.get(&key).await;

    Ok(value.unwrap_or_default())
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<String, LinearizableReadError> {
    app.ensure_linearizable().await?;
    let value = app.data.get(&key).await;

    Ok(value.unwrap_or_default())
}
