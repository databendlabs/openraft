use std::sync::Arc;

use app_http::FollowerReadError;

use crate::app::App;
use crate::typ::*;

/**
 * Application API
 *
 * This is where you place your application, you can use the example below to create your
 * API. The current implementation:
 *
 *  - `POST - /read` attempt to find a value from a given key.
 */
pub async fn read(app: Arc<App>, key: String) -> Result<String, Infallible> {
    let value = app.data.get(&key).await;

    Ok(value.unwrap_or_default())
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<String, LinearizableReadError> {
    app.ensure_linearizable().await?;
    let value = app.data.get(&key).await;

    Ok(value.unwrap_or_default())
}

pub async fn follower_read(app: Arc<App>, key: String) -> Result<String, FollowerReadError> {
    app.ensure_follower_read_ready().await?;
    let value = app.data.get(&key).await;

    Ok(value.unwrap_or_default())
}
