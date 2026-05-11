// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use axum::{
    routing::get, 
    Router, 
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    extract::{State, Request},
    http::{StatusCode, HeaderMap},
};
use std::sync::Arc;
use std::net::SocketAddr;
use crate::scheduler::Scheduler;
use crate::db::Db;
use tower_http::services::ServeDir;
use tower_sessions::{SessionManagerLayer, Expiry, Session};
use crate::web::login::USER_SESSION_KEY;

pub mod login;
mod handlers;

pub struct WebServer {
    scheduler: Arc<Scheduler>,
    db: Db,
    addr: SocketAddr,
}

impl WebServer {
    pub fn new(scheduler: Arc<Scheduler>, db: Db, port: u16) -> Self {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        Self { scheduler, db, addr }
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let env = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "production".to_string());
        let is_production = env == "production";
        let is_https = std::env::var("GITHUB_REDIRECT_URL").map(|u| u.starts_with("https")).unwrap_or(false);
        
        let session_layer = SessionManagerLayer::new(self.db)
            .with_secure(is_https || is_production) // Force secure in production if https is intended
            .with_same_site(if is_production { tower_sessions::cookie::SameSite::Strict } else { tower_sessions::cookie::SameSite::Lax })
            .with_expiry(Expiry::OnInactivity(tower_sessions::cookie::time::Duration::days(7)));

        let scheduler_clone = Arc::clone(&self.scheduler);

        // API and Cluster Routes (Protected by Session OR API Key)
        let api_routes = Router::new()
            .route("/task-status", get(handlers::status_page_handler))
            .route("/workers", get(handlers::workers_page_handler))
            .route("/webhook-status", get(handlers::webhook_status_page_handler))
            .route("/cluster-logs", get(handlers::task_logs_page_handler))
            .route("/admin/reload", get(handlers::reload_config_handler))
            .route("/tasks", get(handlers::get_tasks_handler))
            .route("/api/cluster-logs", get(handlers::get_task_logs_handler))
            .route("/tasks/:id/toggle", axum::routing::post(handlers::toggle_task_handler))
            .route("/tasks/:id/run", axum::routing::post(handlers::run_task_handler))
            .route("/webhooks/:id", axum::routing::delete(handlers::delete_webhook_handler))
            .route("/logs/:id", get(handlers::get_run_logs_handler))
            .route("/events", get(handlers::events_handler))
            .layer(middleware::from_fn_with_state(scheduler_clone.clone(), auth_middleware));

        let app = Router::new()
            .route("/", get(login::login_page_handler))
            .route("/auth/github", get(login::github_login))
            .route("/auth/github/callback", get(login::github_callback))
            .route("/logout", get(login::logout))
            .route("/api/rpc", axum::routing::post(handlers::task_rpc_handler))
            .merge(api_routes)
            .route("/webhook", axum::routing::any(handlers::webhook_handler))
            .route("/webhook/*path", axum::routing::any(handlers::webhook_handler))
            .route("/ws/worker", get(handlers::worker_websocket_handler))
            .nest_service("/static", ServeDir::new("web"))
            .layer(session_layer)
            .with_state(self.scheduler);

        println!("Web server listening on http://{}", self.addr);
        let listener = tokio::net::TcpListener::bind(self.addr).await? ;
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
        Ok(())
    }
}

async fn auth_middleware(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    // 0. Always allow RPC endpoint (it has its own token auth)
    let path = request.uri().path();
    if path == "/api/rpc" {
        return next.run(request).await;
    }

    // 1. Check Session
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_some() {
        return next.run(request).await;
    }

    // 2. Check API Key
    let api_key = headers.get("X-API-Key").and_then(|h| h.to_str().ok());
    if let Some(key) = api_key {
        if scheduler.verify_api_key(key) {
            return next.run(request).await;
        }
    }

    // 3. Unauthorized
    let path = request.uri().path();
    if path.starts_with("/api/") || path == "/events" || path.contains("/toggle") || path.contains("/run") || request.method() == axum::http::Method::DELETE {
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    } else {
        // For pages, redirect to login
        Redirect::to("/").into_response()
    }
}
