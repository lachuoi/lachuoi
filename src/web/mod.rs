use axum::{routing::get, Router};
use std::sync::Arc;
use std::net::SocketAddr;
use crate::scheduler::Scheduler;
use tower_http::services::ServeDir;

mod handlers;

pub struct WebServer {
    scheduler: Arc<Scheduler>,
    addr: SocketAddr,
}

impl WebServer {
    pub fn new(scheduler: Arc<Scheduler>, port: u16) -> Self {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        Self { scheduler, addr }
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let app = Router::new()
            .route("/", get(handlers::login_page_handler))
            .route("/task-status", get(handlers::status_page_handler))
            .route("/tasks", get(handlers::get_tasks_handler))
            .route("/events", get(handlers::events_handler))
            .nest_service("/static", ServeDir::new("web"))
            .with_state(self.scheduler);

        println!("Web server listening on http://{}", self.addr);
        let listener = tokio::net::TcpListener::bind(self.addr).await? ;
        axum::serve(listener, app).await?;
        Ok(())
    }
}
