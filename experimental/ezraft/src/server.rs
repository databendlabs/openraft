//! HTTP server for EzRaft
//!
//! This module provides the HTTP server that handles:
//! - Internal Raft RPC (append entries, vote)
//! - Admin API (join, change membership, metrics)

use std::io::Cursor;

use actix_web::App;
use actix_web::HttpServer;
use actix_web::web;
use actix_web::web::Data;
use openraft::BasicNode;
use openraft::ChangeMembers;
use openraft::Snapshot;
use openraft::errors::Infallible;
use openraft::errors::decompose::DecomposeResult;
use openraft::raft;
use openraft::raft::SnapshotResponse;
use serde::Deserialize;

use crate::app::EzApp;
use crate::network::SnapshotTransfer;
use crate::raft::EzRaft;
use crate::type_config::OpenRaftTypes;

/// Type alias for OpenRaft types
type C<T> = OpenRaftTypes<T>;

/// HTTP server wrapper for EzRaft
///
/// This is both a working server and a sample to copy. The `/raft/*` routes and the admin
/// handlers are what every deployment needs and can be taken as they are. The application
/// routes cannot be: `POST /api/write` and `GET /api/read?key=...` are the smallest pair that
/// exercises an app, not the API a real service exposes - that one has an endpoint per
/// operation, with the path, parameters and encoding each operation deserves.
///
/// So copy this file into your own crate and rewrite the application handlers there.
/// [`EzRaft::write`] and [`EzRaft::read`] are the only two entry points they need - the first
/// goes through the log, the second reads local state.
pub struct EzServer<T>
where T: EzApp
{
    raft: EzRaft<T>,
}

impl<T> EzServer<T>
where T: EzApp
{
    pub fn new(raft: EzRaft<T>) -> Self {
        Self { raft }
    }

    /// Run the HTTP server
    pub async fn run(self) -> std::io::Result<()> {
        let addr = self.raft.addr().to_string();
        let server_data = Data::new(self);

        let server = HttpServer::new(move || {
            App::new()
                .app_data(server_data.clone())
                // Raft internal RPC
                .route("/raft/append", web::post().to(Self::handle_append))
                .route("/raft/vote", web::post().to(Self::handle_vote))
                .route("/raft/snapshot", web::post().to(Self::handle_snapshot))
                // Application API
                .route("/api/write", web::post().to(Self::handle_write))
                .route("/api/read", web::get().to(Self::handle_read))
                // Admin API
                .route("/api/join", web::post().to(Self::handle_join))
                .route("/api/change_membership", web::post().to(Self::handle_change_membership))
                .route("/api/metrics", web::get().to(Self::handle_metrics))
        })
        .bind(&addr)?;

        server.run().await
    }

    /// Raft append entries RPC handler
    ///
    /// The body is the `Result` the peer's [`crate::network::Network`] expects; only a
    /// [`Fatal`](openraft::errors::Fatal) error becomes an HTTP error status.
    async fn handle_append(
        req: web::Json<raft::AppendEntriesRequest<C<T>>>,
        ez: Data<Self>,
    ) -> Result<web::Json<Result<raft::AppendEntriesResponse<C<T>>, Infallible>>, actix_web::Error> {
        let resp = ez
            .raft
            .inner()
            .append_entries(req.into_inner())
            .await
            .decompose()
            .map_err(|e| actix_web::error::ErrorInternalServerError(format!("append_entries failed: {}", e)))?;

        Ok(web::Json(resp))
    }

    /// Raft vote RPC handler
    async fn handle_vote(
        req: web::Json<raft::VoteRequest<C<T>>>,
        ez: Data<Self>,
    ) -> Result<web::Json<Result<raft::VoteResponse<C<T>>, Infallible>>, actix_web::Error> {
        let resp = ez
            .raft
            .inner()
            .vote(req.into_inner())
            .await
            .decompose()
            .map_err(|e| actix_web::error::ErrorInternalServerError(format!("vote failed: {}", e)))?;

        Ok(web::Json(resp))
    }

    /// Raft install snapshot RPC handler
    ///
    /// A leader falls back to this when a follower lags behind the purged log. The snapshot
    /// arrives whole in a single request.
    async fn handle_snapshot(
        req: web::Json<SnapshotTransfer>,
        ez: Data<Self>,
    ) -> Result<web::Json<Result<SnapshotResponse<C<T>>, Infallible>>, actix_web::Error> {
        let SnapshotTransfer { vote, meta, data } = req.into_inner();
        let snapshot = Snapshot {
            meta,
            snapshot: Cursor::new(data),
        };

        let resp = ez
            .raft
            .inner()
            .install_full_snapshot(vote, snapshot)
            .await
            .map_err(|e| actix_web::error::ErrorInternalServerError(format!("install_snapshot failed: {}", e)))?;

        Ok(web::Json(Ok(resp)))
    }

    /// Application write API handler
    ///
    /// Takes the application's own request type as JSON, runs it through Raft, and returns
    /// whatever the state machine's `apply` produced. This is how a client drives the cluster.
    async fn handle_write(
        req: web::Json<T::Request>,
        ez: Data<Self>,
    ) -> Result<web::Json<T::Response>, actix_web::Error> {
        let resp = ez
            .raft
            .write(req.into_inner())
            .await
            .map_err(|e| actix_web::error::ErrorInternalServerError(format!("write failed: {}", e)))?;

        Ok(web::Json(resp))
    }

    /// Application read API handler
    ///
    /// `GET /api/read?key=...` answers a keyed read from local memory via [`EzApp::read`]: the
    /// write API puts keys in, this reads one back. `key` is required - a request without it is
    /// a 400 - and a key the app does not hold is a 404.
    ///
    /// Reads cost no consensus round and no log entry, and are as fresh as this node's
    /// replication - a read that must be linearizable goes through [`EzRaft::write`] instead.
    async fn handle_read(
        query: web::Query<ReadQuery>,
        ez: Data<Self>,
    ) -> Result<web::Json<serde_json::Value>, actix_web::Error> {
        let key = &query.key;

        let Some(value) = ez.raft.read(|app| app.read(key)).await else {
            return Err(actix_web::error::ErrorNotFound(format!("no value for key {:?}", key)));
        };

        Ok(web::Json(value))
    }

    /// Change membership API handler
    async fn handle_change_membership(
        req: web::Json<ChangeMembers<u64, BasicNode>>,
        ez: Data<Self>,
    ) -> Result<web::Json<serde_json::Value>, actix_web::Error> {
        ez.raft
            .change_membership(req.into_inner())
            .await
            .map_err(|e| actix_web::error::ErrorInternalServerError(format!("change_membership failed: {}", e)))?;

        Ok(web::Json(serde_json::json!({ "status": "ok" })))
    }

    /// Metrics API handler
    async fn handle_metrics(ez: Data<Self>) -> Result<web::Json<openraft::RaftMetrics<C<T>>>, actix_web::Error> {
        let metrics = ez.raft.metrics().await;
        Ok(web::Json(metrics))
    }

    /// Join cluster API handler
    ///
    /// A new node calls this endpoint to join an existing cluster.
    /// The leader assigns a unique node ID based on the log index, adds the node as a learner,
    /// and promotes it to a voter once it has caught up, so that joining a cluster is all it
    /// takes to make the cluster fault-tolerant.
    async fn handle_join(
        req: web::Json<JoinRequest>,
        ez: Data<Self>,
    ) -> Result<web::Json<JoinResponse>, actix_web::Error> {
        let metrics = ez.raft.metrics().await;

        // Check if we're the leader
        if metrics.current_leader != Some(metrics.id) {
            // Not the leader - find leader address and return it
            let leader_addr = metrics.current_leader.and_then(|leader_id| {
                metrics.membership_config.membership().get_node(&leader_id).map(|n| n.addr.clone())
            });
            return Ok(web::Json(Err(leader_addr)));
        }

        // We are the leader - write a blank entry to get a unique log index
        let write_result = ez
            .raft
            .inner()
            .write_blank()
            .await
            .map_err(|e| actix_web::error::ErrorInternalServerError(format!("join write failed: {}", e)))?;

        let node_id = write_result.log_id.index;

        // Add the new node as a learner
        ez.raft
            .add_learner(node_id, req.addr.clone())
            .await
            .map_err(|e| actix_web::error::ErrorInternalServerError(format!("add_learner failed: {}", e)))?;

        // The reconcile loop promotes the learner once it catches up. The response must go out
        // first regardless: the new node cannot replicate anything, and therefore cannot catch
        // up, until it learns its node id.
        Ok(web::Json(Ok(node_id)))
    }
}

/// Run the HTTP server (convenience function)
pub(crate) async fn run<T>(raft: EzRaft<T>) -> std::io::Result<()>
where T: EzApp {
    EzServer::new(raft).run().await
}

/// Query for [`EzServer::handle_read`]
#[derive(Debug, Deserialize)]
struct ReadQuery {
    /// Key to read, passed to [`EzApp::read`]
    key: String,
}

/// Join cluster request
#[derive(Debug, Deserialize)]
struct JoinRequest {
    /// New node's HTTP address
    addr: String,
}

/// Join cluster response: Ok(node_id) or Err(leader_addr)
type JoinResponse = Result<u64, Option<String>>;
