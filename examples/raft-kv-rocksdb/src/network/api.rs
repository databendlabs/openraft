use std::sync::Arc;

use openraft::error::CheckIsLeaderError;
use openraft::error::Infallible;
use tide::Body;
use tide::Request;
use tide::Response;
use tide::StatusCode;

use crate::app::App;
use crate::Node;
use crate::NodeId;
use crate::Server;

pub fn rest(app: &mut Server) {
    let mut api = app.at("/api");
    api.at("/write").post(write);
    api.at("/read").post(read);
    api.at("/consistent_read").post(consistent_read);
}
/**
 * Application API
 *
 * This is where you place your application, you can use the example below to create your
 * API. The current implementation:
 *
 *  - `POST - /write` saves a value in a key and sync the nodes.
 *  - `POST - /read` attempt to find a value from a given key.
 */
async fn write(mut req: Request<Arc<App>>) -> tide::Result {
    let body = req.body_json().await?;
    let tokio_handle = req.state().tokio_handle.clone();
    let raft = req.state().raft.clone();
    let res = tokio_handle
        .spawn(async move { raft.client_write(body).await })
        .await
        .map_err(|e| tide::Error::from_str(StatusCode::InternalServerError, e.to_string()))?;
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

async fn read(mut req: Request<Arc<App>>) -> tide::Result {
    let key: String = req.body_json().await?;
    let kvs = req.state().key_values.read().await;
    let value = kvs.get(&key);

    let res: Result<String, Infallible> = Ok(value.cloned().unwrap_or_default());
    Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
}

async fn consistent_read(mut req: Request<Arc<App>>) -> tide::Result {
    let app = req.state().clone();
    let tokio_handle = app.tokio_handle.clone();
    let raft = app.raft.clone();
    let ret = tokio_handle
        .spawn(async move { raft.ensure_linearizable().await })
        .await
        .map_err(|e| tide::Error::from_str(StatusCode::InternalServerError, e.to_string()))?;

    match ret {
        Ok(_) => {
            let key: String = req.body_json().await?;
            let kvs = app.key_values.read().await;

            let value = kvs.get(&key);

            let res: Result<String, CheckIsLeaderError<NodeId, Node>> = Ok(value.cloned().unwrap_or_default());
            Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&res)?).build())
        }
        e => Ok(Response::builder(StatusCode::Ok).body(Body::from_json(&e)?).build()),
    }
}
