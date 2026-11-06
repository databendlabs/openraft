use std::sync::Arc;

use app_http::FollowerReadError;
use openraft::errors::Infallible;
use openraft::errors::LinearizableReadError;

use crate::TypeConfig;
use crate::app::App;

/// `POST /read`: return the value of a key from this node's state machine, possibly stale.
pub async fn read(app: Arc<App>, key: String) -> Result<types_kv::Response, Infallible> {
    let value = app.data.get(&key).await;

    Ok(types_kv::Response { value })
}

/// `POST /linearizable_read`: confirm the leadership with a quorum, then read on the leader.
pub async fn linearizable_read(
    app: Arc<App>,
    key: String,
) -> Result<types_kv::Response, LinearizableReadError<TypeConfig>> {
    app.ensure_linearizable().await?;
    let value = app.data.get(&key).await;

    Ok(types_kv::Response { value })
}

/// `POST /follower_read`: wait until this node applies what the leader has committed, then read.
pub async fn follower_read(app: Arc<App>, key: String) -> Result<types_kv::Response, FollowerReadError> {
    app.ensure_follower_read_ready().await?;
    let value = app.data.get(&key).await;

    Ok(types_kv::Response { value })
}
