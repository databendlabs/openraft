use std::fs::OpenOptions;
use std::io::prelude::*;
use std::num::NonZeroI32;
use std::time::Duration;

use actix_web::http::StatusCode;
use actix_web::post;
use actix_web::web::Data;
use actix_web::web::Json;
use actix_web::HttpResponse;
use actix_web::HttpResponseBuilder;
use actix_web::Responder;
use actix_web::ResponseError;
use inferno::flamegraph;
use pprof::protos::Message;
use thiserror::Error;

use crate::app::ExampleApp;
// --- Generate profile info, such as flamegraph

pub struct Profiling {
    duration: Duration,
    frequency: i32,
}

#[derive(Error, Debug)]
pub enum ProfileError {
    #[error(transparent)]
    InternalError(#[from] pprof::Error),
}

impl ResponseError for ProfileError {
    fn error_response(&self) -> HttpResponse {
        HttpResponseBuilder::new(self.status_code())
            .insert_header(("Content-Type", "text/html; charset=utf-8"))
            .body(self.to_string())
    }

    fn status_code(&self) -> StatusCode {
        match *self {
            ProfileError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub type ProfileResult<T> = std::result::Result<T, ProfileError>;

impl Profiling {
    pub fn create(duration: Duration, frequency: i32) -> Self {
        Self { duration, frequency }
    }

    pub async fn report(&self) -> ProfileResult<pprof::Report> {
        let guard = pprof::ProfilerGuard::new(self.frequency).map_err(ProfileError::InternalError)?;
        tokio::time::sleep(self.duration).await;
        guard.report().build().map_err(ProfileError::InternalError)
    }

    pub async fn dump_flamegraph(&self) -> ProfileResult<Vec<u8>> {
        let mut body: Vec<u8> = Vec::new();

        let report = self.report().await?;
        let option = &mut flamegraph::Options::default();
        option.font_size = 14;
        option.image_width = Some(2500);
        report.flamegraph_with_options(&mut body, option)?;

        Ok(body)
    }

    pub async fn dump_proto(&self) -> ProfileResult<Vec<u8>> {
        let mut body: Vec<u8> = Vec::new();

        let report = self.report().await?;
        let profile = report.pprof()?;
        let _ret = profile.encode(&mut body);

        Ok(body)
    }
}

/// Dump flamegraph.
#[post("/profile/dump_flamegraph")]
pub async fn dump_flamegraph(
    _app: Data<ExampleApp>,
    req: Json<(u64, NonZeroI32)>,
) -> actix_web::Result<impl Responder> {
    let seconds: u64 = req.0 .0;
    let frequency = req.0 .1;

    let duration = Duration::from_secs(seconds);
    tracing::info!("start pprof request second: {:?} frequency: {:?}", seconds, frequency);
    let profile = Profiling::create(duration, i32::from(frequency));

    let body = profile.dump_flamegraph().await?;

    let filename = "flamegraph.svg";
    let mut file = OpenOptions::new().create(true).write(true).open(filename)?;
    let err = file.write(&body);

    tracing::info!("save pprof flamegraph : {:?} {} in {}", err, body.len(), filename);
    Ok(Json("OK"))
}
