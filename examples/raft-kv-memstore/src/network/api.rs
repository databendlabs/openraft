use std::sync::Arc;

use client_http::FollowerReadError;
use openraft::ReadPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::decompose::DecomposeResult;
use openraft::raft::linearizable_read::Linearizer;

use crate::app::App;
use crate::typ::*;

/**
 * Application API
 *
 * This is where you place your application, you can use the example below to create your
 * API. The current implementation:
 *
 *  - `POST - /write` saves a value in a key and sync the nodes.
 *  - `POST - /read` attempt to find a value from a given key.
 */
pub async fn write(app: Arc<App>, req: types_kv::Request) -> Result<ClientWriteResponse, ClientWriteError> {
    app.raft.client_write(req).await.decompose().unwrap()
}

pub async fn read(app: Arc<App>, key: String) -> Result<String, Infallible> {
    let value = app.state_machine_store.get(&key).await;

    Ok(value.unwrap_or_default())
}

pub async fn linearizable_read(app: Arc<App>, key: String) -> Result<String, LinearizableReadError> {
    let ret = app.raft.get_read_linearizer(ReadPolicy::ReadIndex).await.decompose().unwrap();

    match ret {
        Ok(linearizer) => {
            linearizer.await_ready(&app.raft).await.unwrap();
            let value = app.state_machine_store.get(&key).await;

            Ok(value.unwrap_or_default())
        }
        Err(e) => Err(e),
    }
}

pub async fn follower_read(app: Arc<App>, key: String) -> Result<String, FollowerReadError> {
    let Some(leader_id) = app.raft.current_leader().await else {
        return Err(FollowerReadError {
            message: "No leader available".to_string(),
        });
    };

    let metrics = app.raft.metrics().borrow_watched().clone();
    let Some(leader_node) = metrics.membership_config.membership().get_node(&leader_id) else {
        return Err(FollowerReadError {
            message: format!("Leader node {} not found in membership", leader_id),
        });
    };

    let client = reqwest::Client::new();
    let url = format!("http://{}/get_linearizer", leader_node.data);
    let response = client.get(&url).send().await.map_err(|e| FollowerReadError {
        message: format!("Failed to contact leader: {}", e),
    })?;

    let linearizer_data: Result<crate::network::management::LinearizerData, LinearizableReadError> =
        response.json().await.map_err(|e| FollowerReadError {
            message: format!("Failed to parse linearizer data: {}", e),
        })?;

    let linearizer_data = linearizer_data.map_err(|e| FollowerReadError {
        message: format!("Leader returned error: {:?}", e),
    })?;

    let linearizer = Linearizer::new(
        linearizer_data.node_id,
        linearizer_data.read_log_id,
        linearizer_data.applied,
    );

    linearizer.await_ready(&app.raft).await.map_err(|e| FollowerReadError {
        message: format!("Failed to wait for state machine: {:?}", e),
    })?;

    let value = app.state_machine_store.get(&key).await;

    Ok(value.unwrap_or_default())
}
