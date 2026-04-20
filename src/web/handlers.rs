use axum::{Json, extract::{State, Query}, response::{Html, Redirect, IntoResponse}, response::sse::{Event, Sse}, http::StatusCode};
use std::sync::Arc;
use crate::scheduler::Scheduler;
use crate::task::TaskStatus;
use tokio_stream::StreamExt;
use futures_util::Stream;
use std::convert::Infallible;
use tower_sessions::Session;
use serde::{Deserialize, Serialize};
use rand::distr::{Alphanumeric, SampleString};

const USER_SESSION_KEY: &str = "user_github_id";

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    code: String,
    state: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GitHubUser {
    id: i64,
    login: String,
}

#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
}

pub async fn github_login(session: Session) -> impl IntoResponse {
    let client_id = std::env::var("GITHUB_CLIENT_ID").expect("GITHUB_CLIENT_ID not set");
    let redirect_url = std::env::var("GITHUB_REDIRECT_URL").expect("GITHUB_REDIRECT_URL not set");
    
    let state = Alphanumeric.sample_string(&mut rand::rng(), 32);

    session.insert("csrf_token", state.clone()).await.unwrap();

    let auth_url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&state={}&scope=user:email",
        client_id,
        urlencoding::encode(&redirect_url),
        urlencoding::encode(&state)
    );

    Redirect::to(&auth_url)
}

pub async fn github_callback(
    Query(query): Query<AuthRequest>,
    session: Session,
) -> impl IntoResponse {
    let csrf_token: Option<String> = session.get("csrf_token").await.unwrap();
    
    if csrf_token.is_none() {
        return (StatusCode::BAD_REQUEST, "Invalid state: session token missing. Ensure cookies are enabled and you are using the same browser tab.").into_response();
    }
    
    if query.state != csrf_token.unwrap() {
        return (StatusCode::BAD_REQUEST, "Invalid state: token mismatch.").into_response();
    }

    let client_id = std::env::var("GITHUB_CLIENT_ID").expect("GITHUB_CLIENT_ID not set");
    let client_secret = std::env::var("GITHUB_CLIENT_SECRET").expect("GITHUB_CLIENT_SECRET not set");

    let client = reqwest::Client::new();
    
    // Exchange code for token
    let token_resp = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", query.code.as_str()),
        ])
        .send()
        .await;

    let token_data: GitHubTokenResponse = match token_resp {
        Ok(resp) => match resp.json().await {
            Ok(data) => data,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to parse token response: {}", e)).into_response(),
        },
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Token request failed: {}", e)).into_response(),
    };

    // Fetch user info
    let user_info: GitHubUser = match client
        .get("https://api.github.com/user")
        .header("User-Agent", "task-scheduler")
        .header("Authorization", format!("Bearer {}", token_data.access_token))
        .send()
        .await {
            Ok(resp) => match resp.json().await {
                Ok(u) => u,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to parse user info: {}", e)).into_response(),
            },
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to fetch user info: {}", e)).into_response(),
        };

    session.insert(USER_SESSION_KEY, user_info.id).await.unwrap();
    session.insert("github_login", user_info.login).await.unwrap();

    Redirect::to("/task-status").into_response()
}

pub async fn logout(session: Session) -> impl IntoResponse {
    session.clear().await;
    Redirect::to("/")
}

pub async fn events_handler(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, impl IntoResponse> {
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_none() {
        return Err(Redirect::to("/"));
    }
    let log_rx = scheduler.subscribe_logs();
    let status_rx = scheduler.subscribe_status();
    
    let initial_msg = futures_util::stream::once(async {
        Ok(Event::default().data("Log stream connected"))
    });

    let log_stream = tokio_stream::wrappers::BroadcastStream::new(log_rx)
        .map(|msg| {
            match msg {
                Ok(text) => Ok(Event::default().event("log").data(text)),
                Err(_) => Ok(Event::default().event("log").data("... (log buffer overflowed)")),
            }
        });

    let status_stream = tokio_stream::wrappers::BroadcastStream::new(status_rx)
        .map(|msg| {
            match msg {
                Ok(status) => {
                    let json = serde_json::to_string(&status).unwrap_or_default();
                    Ok(Event::default().event("status").data(json))
                },
                Err(_) => Ok(Event::default().event("log").data("... (status buffer overflowed)")),
            }
        });

    let combined = StreamExt::merge(initial_msg.chain(log_stream), status_stream);

    Ok(Sse::new(combined).keep_alive(axum::response::sse::KeepAlive::default()))
}

pub async fn get_tasks_handler(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> Result<Json<Vec<TaskStatus>>, Redirect> {
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_none() {
        return Err(Redirect::to("/"));
    }
    Ok(Json(scheduler.get_tasks_status().await))
}

pub async fn login_page_handler(session: Session) -> impl IntoResponse {
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_some() {
        return Redirect::to("/task-status").into_response();
    }
    match std::fs::read_to_string("web/templates/login.html") {
        Ok(t) => Html(t).into_response(),
        Err(e) => Html(format!("Error loading login template: {}", e)).into_response(),
    }
}

pub async fn reload_config_handler(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> impl IntoResponse {
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    match scheduler.reload_from_file("cron.toml").await {
        Ok(_) => (StatusCode::OK, "Configuration reloaded successfully").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reload configuration: {}", e)).into_response(),
    }
}

pub async fn status_page_handler(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> impl IntoResponse {
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_none() {
        return Redirect::to("/").into_response();
    }
    let tasks = scheduler.get_tasks_status().await;
    
    let mut rows = String::new();
    for task in tasks {
        let mut status_class = if task.enabled { "status-enabled" } else { "status-disabled" };
        let mut status_text = if task.enabled { "Active" } else { "Paused" };

        if task.enabled && task.last_failed {
            status_class = "status-disabled";
            status_text = "Failed";
        }

        let last_run = format_relative_time(&task.last_run);
        let duration = task.last_duration_ms.map(|ms| format!("<span class='duration'>({}ms)</span>", ms)).unwrap_or_default();
        
        rows.push_str(&format!(
            "<tr>
                <td><strong>{name}</strong></td>
                <td><span class='badge type-badge'>{t_type}</span></td>
                <td><code>{cron}</code></td>
                <td>{tz}</td>
                <td class='time-cell'>{last} {dur}</td>
                <td><span class='status-pill {s_class}'>{s_text}</span></td>
            </tr>",
            name = task.name,
            t_type = task.task_type,
            cron = task.cron,
            tz = task.timezone,
            last = last_run,
            dur = duration,
            s_class = status_class,
            s_text = status_text
        ));
    }

    // Load template from file
    let template = match std::fs::read_to_string("web/templates/status.html") {
        Ok(t) => t,
        Err(e) => return Html(format!("Error loading template: {}", e)).into_response(),
    };

    let github_login: String = session.get("github_login").await.unwrap().unwrap_or_else(|| "Unknown".to_string());
    Html(template.replace("{{rows}}", &rows).replace("{{user}}", &github_login)).into_response()
}

fn format_relative_time(last_run_rfc3339: &Option<String>) -> String {
    let last_run = match last_run_rfc3339 {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(dt) => dt.with_timezone(&chrono::Utc),
            Err(_) => return "Invalid date".to_string(),
        },
        None => return "Never".to_string(),
    };

    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(last_run);

    if duration.num_seconds() < 0 {
        return "Just now".to_string();
    }

    if duration.num_days() >= 7 {
        return last_run.format("%Y-%m-%d %H:%M:%S").to_string();
    }

    if duration.num_days() >= 1 {
        return format!("{} days ago", duration.num_days());
    }

    if duration.num_hours() >= 1 {
        return format!("{} hours ago", duration.num_hours());
    }

    if duration.num_minutes() >= 1 {
        return format!("{} minutes ago", duration.num_minutes());
    }

    if duration.num_seconds() <= 5 {
        return "Just now".to_string();
    }

    format!("{} seconds ago", duration.num_seconds())
}
