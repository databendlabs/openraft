use actix_web::post;
use actix_web::web;
use actix_web::web::Data;
use actix_web::Responder;
use openraft::raft::ClientWriteRequest;
use openraft::raft::EntryPayload;
use web::Json;

use crate::app::ExampleApp;
use crate::store::ExampleRequest;

/**
 * Application API
 *
 * This is where you place your application, you can use the example below to create your
 * API. The current implementation:
 *
 *  - `POST - /write` saves a value in a key and sync the nodes.
 *  - `GET - /read` attempt to find a value from a given key.
 */
#[post("/write")]
pub async fn write(app: Data<ExampleApp>, req: Json<ExampleRequest>) -> actix_web::Result<impl Responder> {
    let request = ClientWriteRequest::new(EntryPayload::Normal(req.0));
    let response = app.raft.client_write(request).await;
    Ok(Json(response))
}

#[post("/read")]
pub async fn read(app: Data<ExampleApp>, req: Json<String>) -> actix_web::Result<impl Responder> {
    let state_machine = app.store.state_machine.read().await;
    let key = req.0;
    let value = state_machine.data.get(&key).cloned();
    Ok(Json(value.unwrap_or_default()))
}
