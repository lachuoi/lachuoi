use axum::{routing::get, Router};
use std::sync::Arc;
use std::net::SocketAddr;
use crate::scheduler::Scheduler;
use crate::db::Db;
use tower_http::services::ServeDir;
use tower_sessions::{SessionManagerLayer, Expiry};

#[path = "web/handlers.rs"]
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
        let is_https = std::env::var("GITHUB_REDIRECT_URL").map(|u| u.starts_with("https")).unwrap_or(false);
        let session_layer = SessionManagerLayer::new(self.db)
            .with_secure(is_https)
            .with_same_site(tower_sessions::cookie::SameSite::Lax)
            .with_expiry(Expiry::OnInactivity(tower_sessions::cookie::time::Duration::days(7)));

        let app = Router::new()
            .route("/", get(handlers::login_page_handler))
            .route("/auth/github", get(handlers::github_login))
            .route("/auth/github/callback", get(handlers::github_callback))
            .route("/logout", get(handlers::logout))
            .route("/task-status", get(handlers::status_page_handler))
            .route("/admin/reload", get(handlers::reload_config_handler))
            .route("/tasks", get(handlers::get_tasks_handler))
            .route("/tasks/:id/toggle", axum::routing::post(handlers::toggle_task_handler))
            .route("/events", get(handlers::events_handler))
            .nest_service("/static", ServeDir::new("web"))
            .layer(session_layer)
            .with_state(self.scheduler);

        println!("Web server listening on http://{}", self.addr);
        let listener = tokio::net::TcpListener::bind(self.addr).await? ;
        axum::serve(listener, app).await?;
        Ok(())
    }
}
