use std::sync::Arc;

use openraft::errors::Infallible;

use crate::app::App;

/// `POST /read`: return the value of a key from this node's state machine, possibly stale.
pub async fn read(app: Arc<App>, key: String) -> Result<types_kv::Response, Infallible> {
    let value = app.data.get(&key).await;

    Ok(types_kv::Response { value })
}
