use actix_web::post;
use actix_web::web;
use actix_web::web::Data;
use actix_web::Responder;
use client_http::FollowerReadError;
use openraft::error::decompose::DecomposeResult;
use openraft::error::CheckIsLeaderError;
use openraft::error::Infallible;
use openraft::raft::linearizable_read::Linearizer;
use openraft::ReadPolicy;
use web::Json;

use crate::app::App;
use crate::store::Request;
use crate::TypeConfig;

/**
 * Application API
 *
 * This is where you place your application, you can use the example below to create your
 * API. The current implementation:
 *
 *  - `POST - /write` saves a value in a key and sync the nodes.
 *  - `POST - /read` attempt to find a value from a given key.
 */
#[post("/write")]
pub async fn write(app: Data<App>, req: Json<Request>) -> actix_web::Result<impl Responder> {
    let response = app.raft.client_write(req.0).await.decompose().unwrap();
    Ok(Json(response))
}

#[post("/read")]
pub async fn read(app: Data<App>, req: Json<String>) -> actix_web::Result<impl Responder> {
    let state_machine = app.state_machine_store.state_machine.read().await;
    let key = req.0;
    let value = state_machine.data.get(&key).cloned();

    let res: Result<String, Infallible> = Ok(value.unwrap_or_default());
    Ok(Json(res))
}

#[post("/linearizable_read")]
pub async fn linearizable_read(app: Data<App>, req: Json<String>) -> actix_web::Result<impl Responder> {
    let ret = app.raft.get_read_linearizer(ReadPolicy::ReadIndex).await.decompose().unwrap();

    match ret {
        Ok(linearizer) => {
            linearizer.await_ready(&app.raft).await.unwrap();

            let state_machine = app.state_machine_store.state_machine.read().await;
            let key = req.0;
            let value = state_machine.data.get(&key).cloned();

            let res: Result<String, CheckIsLeaderError<TypeConfig>> = Ok(value.unwrap_or_default());
            Ok(Json(res))
        }
        Err(e) => Ok(Json(Err(e))),
    }
}

/// Perform a linearizable read on a follower by obtaining a linearizer from the leader
///
/// This demonstrates how to distribute read load across followers while maintaining
/// linearizability guarantees:
/// 1. Get current leader from local metrics
/// 2. Fetch linearizer from leader via HTTP
/// 3. Wait for local state machine to catch up to the linearizer's read_log_id
/// 4. Read from local state machine
#[post("/follower_read")]
pub async fn follower_read(app: Data<App>, req: Json<String>) -> actix_web::Result<impl Responder> {
    // 1. Get current leader
    let leader_id = match app.raft.current_leader().await {
        Some(id) => id,
        None => {
            return Ok(Json(Err(FollowerReadError {
                message: "No leader available".to_string(),
            })));
        }
    };

    // 2. Get leader's address from membership config
    let metrics = app.raft.metrics().borrow().clone();
    let leader_node = match metrics.membership_config.membership().get_node(&leader_id) {
        Some(node) => node,
        None => {
            return Ok(Json(Err(FollowerReadError {
                message: format!("Leader node {} not found in membership", leader_id),
            })));
        }
    };
    let leader_addr = &leader_node.addr;

    // 3. Get linearizer from leader via HTTP
    let client = reqwest::Client::new();
    let url = format!("http://{}/get_linearizer", leader_addr);

    let response = match client.post(&url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            return Ok(Json(Err(FollowerReadError {
                message: format!("Failed to contact leader: {}", e),
            })));
        }
    };

    let linearizer_data_result: Result<crate::network::management::LinearizerData, CheckIsLeaderError<TypeConfig>> =
        match response.json().await {
            Ok(result) => result,
            Err(e) => {
                return Ok(Json(Err(FollowerReadError {
                    message: format!("Failed to parse linearizer data: {}", e),
                })));
            }
        };

    let linearizer_data = match linearizer_data_result {
        Ok(data) => data,
        Err(e) => {
            return Ok(Json(Err(FollowerReadError {
                message: format!("Leader returned error: {:?}", e),
            })));
        }
    };

    // Reconstruct linearizer from the data
    let linearizer = Linearizer::new(
        linearizer_data.node_id,
        linearizer_data.read_log_id,
        linearizer_data.applied,
    );

    // 4. Wait for local state machine to catch up
    if let Err(e) = linearizer.await_ready(&app.raft).await {
        return Ok(Json(Err(FollowerReadError {
            message: format!("Failed to wait for state machine: {:?}", e),
        })));
    }

    // 5. Read from local state machine
    let state_machine = app.state_machine_store.state_machine.read().await;
    let key = req.0;
    let value = state_machine.data.get(&key).cloned();

    Ok(Json(Ok(value.unwrap_or_default())))
}
