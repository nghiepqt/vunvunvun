#![forbid(unsafe_code)]
#![allow(clippy::disallowed_types)]

use axum::{Router, routing::get};
use ef_protocol::AiTier;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app: Router = Router::new().route("/healthz", get(|| async { "ok" }));
    let _default_tier = AiTier::T2;

    tracing::info!("ef-gateway router initialized");
    let _ = app;
}
