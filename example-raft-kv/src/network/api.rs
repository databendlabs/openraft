use actix_web::HttpResponse;
use actix_web::post;
use actix_web::web;
use actix_web::web::Data;
use actix_web::Responder;
use openraft::NodeId;
use openraft::error::ClientWriteError;
use openraft::error::ForwardToLeader;
use openraft::raft::ClientWriteRequest;
use openraft::raft::ClientWriteResponse;
use openraft::raft::EntryPayload;
use serde::Serialize;
use web::Json;

use crate::app::ExampleApp;
use crate::store::ExampleRequest;
use crate::store::ExampleResponse;

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

    // Redirect this call to the leader, this is required because a `Candidate`, `Follower` or `Learner` is not allowed to write data to
    // other nodes. The payload has to wrapped around the type fo expect payload that raft uses. In this case it is real payload and
    // `EntryPayload::Normal(D)` will be used.
    let request: ClientWriteRequest<ExampleRequest> = ClientWriteRequest::new(EntryPayload::Normal(req.0.clone()));
    
    // Attempt to write the data to the raft and dispatch to the nodes.
    let response: Result<ClientWriteResponse<ExampleResponse>, ClientWriteError> = app.raft.client_write(request).await;

    match response {
        Ok(_) => Ok(HttpResponse::Ok().finish()),
        Err(error) => {
            match error {
                ClientWriteError::ForwardToLeader(foward_to_leader) => 
                    redirect_to_leader(&app, &req, "/write", foward_to_leader).await,
                _ => Ok(HttpResponse::BadRequest().body("Failed write number"))
            }
        }
    }
}

async fn redirect_to_leader<T: Serialize>(
  app: &Data<ExampleApp>,
  req: &Json<T>,
  route: &str,
  foward_to_leader: ForwardToLeader,
) -> actix_web::Result<HttpResponse> {

  // In a real world application you should not unwrap this, instead return a error related to the missing leader id.
  let leader_id: NodeId = foward_to_leader.leader_id.unwrap();

  let nodes = {
      let state_machine = app.store.state_machine.read().await;
      state_machine.nodes.clone()
  };

  match nodes.get(&leader_id) {
      Some(ip) => {
          let response = app.client
              .post(format!("http://{}{}", ip, route))
              .json(&req)
              .send()
              .await;
          Ok(if response.is_ok() { 
              HttpResponse::Ok().finish()
          } else { 
              HttpResponse::BadRequest().body("Failed write number")
          })
      }
      None => Ok(HttpResponse::BadRequest().body("Failed write number"))
  }
  
}

#[post("/read")]
pub async fn read(app: Data<ExampleApp>, req: Json<String>) -> actix_web::Result<impl Responder> {
    let state_machine = app.store.state_machine.read().await;
    let key = req.0;
    let value = state_machine.data.get(&key).cloned();
    Ok(Json(value.unwrap_or_default()))
}
