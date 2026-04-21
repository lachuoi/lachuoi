use axum::{Json, extract::{State, Path}, response::{Html, Redirect, IntoResponse}, response::sse::{Event, Sse}, http::{StatusCode, Method, HeaderMap}};
use std::sync::Arc;
use uuid::Uuid;
use crate::scheduler::Scheduler;
use crate::task::TaskStatus;
use tokio_stream::StreamExt;
use futures_util::Stream;
use std::convert::Infallible;
use tower_sessions::Session;
use serde::Deserialize;
use crate::web::login::USER_SESSION_KEY;

pub async fn webhook_handler(
    State(scheduler): State<Arc<Scheduler>>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let db = scheduler.get_db();
    let path = uri.path().to_string();
    let method_str = method.to_string();
    
    let mut headers_map = std::collections::HashMap::new();
    for (name, value) in headers.iter() {
        if let Ok(value_str) = value.to_str() {
            headers_map.insert(name.to_string(), value_str.to_string());
        }
    }
    let headers_json = serde_json::to_string(&headers_map).unwrap_or_default();

    match db.save_webhook(&path, &method_str, &headers_json, &body).await {
        Ok(_) => (StatusCode::OK, "OK"),
        Err(e) => {
            eprintln!("Failed to save webhook: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
        }
    }
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

#[derive(Deserialize)]
pub struct ToggleRequest {
    pub enabled: bool,
}

pub async fn toggle_task_handler(
    State(scheduler): State<Arc<Scheduler>>,
    Path(task_id): Path<Uuid>,
    session: Session,
    Json(payload): Json<ToggleRequest>,
) -> impl IntoResponse {
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    scheduler.set_task_enabled(task_id, payload.enabled).await;
    StatusCode::OK.into_response()
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
        let mut status_class = if task.enabled { 
            "bg-green-50 text-green-700 border-green-200 dark:bg-green-900/20 dark:text-green-400 dark:border-green-800" 
        } else { 
            "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/20 dark:text-red-400 dark:border-red-800" 
        };
        let mut status_text = if task.enabled { "Active" } else { "Paused" };

        if task.enabled && task.last_failed {
            status_class = "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/20 dark:text-red-400 dark:border-red-800";
            status_text = "Failed";
        }

        let last_run = format_relative_time(&task.last_run);
        let duration = task.last_duration_ms.map(|ms| format!("<span class='ml-1 text-xs text-blue-600 dark:text-blue-400 font-bold'>({}ms)</span>", ms)).unwrap_or_default();
        
        let toggle_btn = if task.enabled {
            format!("<button class='px-3 py-1 text-xs font-semibold text-red-600 bg-red-50 border border-red-200 rounded-md hover:bg-red-600 hover:text-white dark:bg-red-900/20 dark:border-red-800 dark:hover:bg-red-600 transition-colors' onclick='toggleTask(\"{}\", false)'>Disable</button>", task.id)
        } else {
            format!("<button class='px-3 py-1 text-xs font-semibold text-green-600 bg-green-50 border border-green-200 rounded-md hover:bg-green-600 hover:text-white dark:bg-green-900/20 dark:border-green-800 dark:hover:bg-green-600 transition-colors' onclick='toggleTask(\"{}\", true)'>Enable</button>", task.id)
        };

        rows.push_str(&format!(
            "<tr class='border-b border-gray-100 dark:border-slate-800 hover:bg-gray-50 dark:hover:bg-slate-800/50 transition-colors'>
                <td class='px-4 py-3 align-middle text-sm font-bold text-gray-900 dark:text-slate-100'>{name}</td>
                <td class='px-4 py-3 align-middle text-xs'><span class='bg-gray-100 dark:bg-slate-800 text-gray-600 dark:text-slate-400 px-2 py-1 rounded font-medium uppercase tracking-wider'>{t_type}</span></td>
                <td class='px-4 py-3 align-middle font-mono text-blue-600 dark:text-blue-400 text-xs'>{cron}</td>
                <td class='px-4 py-3 align-middle text-gray-600 dark:text-slate-400 text-sm'>{tz}</td>
                <td class='px-4 py-3 align-middle text-sm text-gray-500 dark:text-slate-500' title='{raw_run}'>{last} {dur}</td>
                <td class='px-4 py-3 align-middle'><span class='px-2 py-1 text-[10px] uppercase font-bold rounded-full border {s_class}'>{s_text}</span></td>
                <td class='px-4 py-3 align-middle'>{btn}</td>
            </tr>",
            name = task.name,
            t_type = task.task_type,
            cron = task.cron,
            tz = task.timezone,
            last = last_run,
            raw_run = task.last_run.as_deref().unwrap_or("Never"),
            dur = duration,
            s_class = status_class,
            s_text = status_text,
            btn = toggle_btn
        ));
    }

    // Load template from file
    let template = match std::fs::read_to_string("web/templates/status.html") {
        Ok(t) => t,
        Err(e) => return Html(format!("Error loading template: {}", e)).into_response(),
    };

    let github_login: String = session.get("github_login").await.unwrap().unwrap_or_else(|| "Unknown".to_string());
    let github_avatar_url: Option<String> = session.get("github_avatar_url").await.unwrap();
    let user_initial = github_login.chars().next().unwrap_or('U').to_uppercase().to_string();
    
    let user_avatar_html = if let Some(url) = github_avatar_url {
        format!("<img src='{}' class='min-w-[2rem] h-8 rounded-full' alt='{}'>", url, github_login)
    } else {
        format!("<div class='min-w-[2rem] h-8 bg-slate-200 dark:bg-slate-800 rounded-full flex items-center justify-center text-slate-600 dark:text-slate-400 font-bold text-xs'>{}</div>", user_initial)
    };

    Html(template
        .replace("{{rows}}", &rows)
        .replace("{{user}}", &github_login)
        .replace("{{user_avatar}}", &user_avatar_html))
        .into_response()
}

pub async fn webhook_status_page_handler(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> impl IntoResponse {
    if session.get::<i64>(USER_SESSION_KEY).await.unwrap().is_none() {
        return Redirect::to("/").into_response();
    }
    
    let db = scheduler.get_db();
    let webhooks = match db.get_webhooks().await {
        Ok(w) => w,
        Err(e) => return Html(format!("Error fetching webhooks: {}", e)).into_response(),
    };
    
    let mut rows = String::new();
    for webhook in webhooks {
        let headers_attr = webhook.headers.replace("'", "&apos;");
        let body_attr = webhook.body.replace("'", "&apos;");

        rows.push_str(&format!(
            "<tr id='row-{id}' class='border-b border-gray-100 dark:border-slate-800 hover:bg-gray-50 dark:hover:bg-slate-800/50 transition-colors' data-headers='{headers}' data-body='{body}'>
                <td class='px-4 py-3 align-middle text-xs font-mono text-gray-500 dark:text-slate-500'>{id}</td>
                <td class='px-4 py-3 align-middle text-sm text-gray-600 dark:text-slate-400'>{time}</td>
                <td class='px-4 py-3 align-middle'><span class='px-2 py-1 text-[10px] uppercase font-bold rounded bg-blue-50 text-blue-700 border border-blue-200 dark:bg-blue-900/20 dark:text-blue-400 dark:border-blue-800'>{method}</span></td>
                <td class='px-4 py-3 align-middle text-sm font-mono text-gray-900 dark:text-slate-100'>{path}</td>
                <td class='px-4 py-3 align-middle'>
                    <button class='px-3 py-1 text-xs font-semibold text-blue-600 bg-blue-50 border border-blue-200 rounded-md hover:bg-blue-600 hover:text-white dark:bg-blue-900/20 dark:border-blue-800 dark:hover:bg-blue-600 transition-colors' onclick='showDetails({id})'>View Details</button>
                </td>
            </tr>",
            id = webhook.id,
            time = webhook.created_at,
            method = webhook.method,
            path = webhook.path,
            headers = headers_attr,
            body = body_attr
        ));
    }

    let template = match std::fs::read_to_string("web/templates/webhook_status.html") {
        Ok(t) => t,
        Err(e) => return Html(format!("Error loading template: {}", e)).into_response(),
    };

    let github_login: String = session.get("github_login").await.unwrap().unwrap_or_else(|| "Unknown".to_string());
    let github_avatar_url: Option<String> = session.get("github_avatar_url").await.unwrap();
    let user_initial = github_login.chars().next().unwrap_or('U').to_uppercase().to_string();
    
    let user_avatar_html = if let Some(url) = github_avatar_url {
        format!("<img src='{}' class='min-w-[2rem] h-8 rounded-full' alt='{}'>", url, github_login)
    } else {
        format!("<div class='min-w-[2rem] h-8 bg-slate-200 dark:bg-slate-800 rounded-full flex items-center justify-center text-slate-600 dark:text-slate-400 font-bold text-xs'>{}</div>", user_initial)
    };

    Html(template
        .replace("{{rows}}", &rows)
        .replace("{{user}}", &github_login)
        .replace("{{user_avatar}}", &user_avatar_html))
        .into_response()
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
