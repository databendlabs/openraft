use actix_web::post;
use actix_web::web::Data;
use actix_web::web::Json;
use actix_web::Responder;
use openraft::error::decompose::DecomposeResult;

use crate::app::App;
use crate::typ::*;

// --- Raft communication

#[post("/vote")]
pub async fn vote(app: Data<App>, req: Json<VoteRequest>) -> actix_web::Result<impl Responder> {
    let res = app.raft.vote(req.0).await.decompose().unwrap();
    Ok(Json(res))
}

#[post("/append")]
pub async fn append(app: Data<App>, req: Json<AppendEntriesRequest>) -> actix_web::Result<impl Responder> {
    let res = app.raft.append_entries(req.0).await.decompose().unwrap();
    Ok(Json(res))
}

#[post("/snapshot")]
pub async fn snapshot(app: Data<App>, req: Json<InstallSnapshotRequest>) -> actix_web::Result<impl Responder> {
    let res = app.raft.install_snapshot(req.0).await.decompose().unwrap();
    Ok(Json(res))
}
