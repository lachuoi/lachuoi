use axum::{Json, extract::State, response::Html, response::sse::{Event, Sse}};
use std::sync::Arc;
use crate::scheduler::Scheduler;
use crate::task::TaskStatus;
use tokio_stream::StreamExt;
use futures_util::Stream;
use std::convert::Infallible;

pub async fn events_handler(
    State(scheduler): State<Arc<Scheduler>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
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

    Sse::new(combined).keep_alive(axum::response::sse::KeepAlive::default())
}

pub async fn get_tasks_handler(State(scheduler): State<Arc<Scheduler>>) -> Json<Vec<TaskStatus>> {
    Json(scheduler.get_tasks_status().await)
}

pub async fn login_page_handler() -> Html<String> {
    match std::fs::read_to_string("web/templates/login.html") {
        Ok(t) => Html(t),
        Err(e) => Html(format!("Error loading login template: {}", e)),
    }
}

pub async fn status_page_handler(State(scheduler): State<Arc<Scheduler>>) -> Html<String> {
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
        Err(e) => return Html(format!("Error loading template: {}", e)),
    };

    Html(template.replace("{{rows}}", &rows))
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
