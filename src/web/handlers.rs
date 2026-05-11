// Copyright 2026 Seungjin Kim
// SPDX-License-Identifier: MIT OR Apache-2.0

use axum::{Json, extract::{State, Path}, response::{Html, IntoResponse}, response::sse::{Event, Sse}, http::{StatusCode, Method, HeaderMap}};
use std::sync::Arc;
use uuid::Uuid;
use crate::scheduler::Scheduler;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::ConnectInfo;
use std::net::SocketAddr;
use serde::Deserialize;
use tower_sessions::Session;

#[derive(Deserialize)]
pub struct ToggleRequest {
    pub enabled: bool,
}

pub async fn workers_page_handler(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> impl IntoResponse {
    let workers = scheduler.get_workers().await;
    let mut worker_rows = String::new();
    
    for worker in workers {
        let running_tasks = if worker.running_tasks.is_empty() {
            "<span class='text-slate-400 italic'>None</span>".to_string()
        } else {
            worker.running_tasks.iter()
                .map(|t| format!("<span class='px-2 py-0.5 bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400 rounded-md text-xs font-bold'>{}</span>", t))
                .collect::<Vec<_>>()
                .join(" ")
        };

        let cpu = worker.metrics.as_ref().map(|m| format!("{:.1}%", m.cpu_usage)).unwrap_or_else(|| "-".to_string());
        let load = worker.metrics.as_ref().and_then(|m| {
            if let (Some(one), Some(five), Some(fifteen)) = (m.load_avg_one, m.load_avg_five, m.load_avg_fifteen) {
                Some(format!("{:.2}, {:.2}, {:.2}", one, five, fifteen))
            } else {
                None
            }
        }).unwrap_or_else(|| "-".to_string());
        let mem = worker.metrics.as_ref().map(|m| format!("{:.1}GB / {:.1}GB", m.memory_used as f32 / 1024.0 / 1024.0 / 1024.0, m.memory_total as f32 / 1024.0 / 1024.0 / 1024.0)).unwrap_or_else(|| "-".to_string());
        let disk = worker.metrics.as_ref().map(|m| format!("{:.0}GB / {:.0}GB", m.disk_used as f32 / 1024.0 / 1024.0 / 1024.0, m.disk_total as f32 / 1024.0 / 1024.0 / 1024.0)).unwrap_or_else(|| "-".to_string());
        
        let cpu_percent = worker.metrics.as_ref().map(|m| m.cpu_usage).unwrap_or(0.0);
        let mem_percent = worker.metrics.as_ref().map(|m| m.memory_used as f32 / m.memory_total as f32 * 100.0).unwrap_or(0.0);
        let disk_percent = worker.metrics.as_ref().map(|m| m.disk_used as f32 / m.disk_total as f32 * 100.0).unwrap_or(0.0);

        worker_rows.push_str(&format!(
            "<tr class='border-b border-gray-100 dark:border-slate-800 hover:bg-gray-50 dark:hover:bg-slate-800/50 transition-colors'>
                <td class='px-4 py-3 align-middle text-xs font-mono text-slate-500 dark:text-slate-500 truncate max-w-[100px]' title='{id}'>{id}</td>
                <td class='px-4 py-3 align-middle text-sm font-bold text-gray-900 dark:text-slate-100'>{hostname}</td>
                <td class='px-4 py-3 align-middle text-sm text-slate-600 dark:text-slate-400 font-mono'>{addr}</td>
                <td class='px-4 py-3 align-middle text-center'>
                    <div class='flex flex-col items-center gap-1'>
                        <span class='text-xs font-bold text-slate-600 dark:text-slate-300'>{cpu}</span>
                        <div class='w-16 h-1 bg-gray-100 dark:bg-slate-800 rounded-full overflow-hidden'>
                            <div class='h-full bg-blue-500' style='width: {cpu_percent}%'></div>
                        </div>
                    </div>
                </td>
                <td class='px-4 py-3 align-middle text-center text-xs font-bold text-slate-600 dark:text-slate-300'>{load}</td>
                <td class='px-4 py-3 align-middle text-center'>
                    <div class='flex flex-col items-center gap-1'>
                        <span class='text-xs font-bold text-slate-600 dark:text-slate-300'>{mem}</span>
                        <div class='w-16 h-1 bg-gray-100 dark:bg-slate-800 rounded-full overflow-hidden'>
                            <div class='h-full bg-emerald-500' style='width: {mem_percent}%'></div>
                        </div>
                    </div>
                </td>
                <td class='px-4 py-3 align-middle text-center'>
                    <div class='flex flex-col items-center gap-1'>
                        <span class='text-xs font-bold text-slate-600 dark:text-slate-300'>{disk}</span>
                        <div class='w-16 h-1 bg-gray-100 dark:bg-slate-800 rounded-full overflow-hidden'>
                            <div class='h-full bg-amber-500' style='width: {disk_percent}%'></div>
                        </div>
                    </div>
                </td>
                <td class='px-4 py-3 align-middle flex flex-wrap gap-1'>{tasks}</td>
                <td class='px-4 py-3 align-middle'><span class='px-2 py-1 text-[10px] uppercase font-bold rounded-full bg-green-50 text-green-700 border border-green-200 dark:bg-green-900/20 dark:text-green-400 dark:border-green-800'>Online</span></td>
            </tr>",
            id = worker.id,
            hostname = worker.hostname,
            addr = worker.addr,
            cpu = cpu,
            cpu_percent = cpu_percent,
            load = load,
            mem = mem,
            mem_percent = mem_percent,
            disk = disk,
            disk_percent = disk_percent,
            tasks = running_tasks
        ));
    }

    if worker_rows.is_empty() {
        worker_rows = "<tr><td colspan='9' class='px-4 py-8 text-center text-slate-500 dark:text-slate-400 italic'>No workers connected</td></tr>".to_string();
    }

    let template = match tokio::fs::read_to_string("web/templates/workers.html").await {
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
        .replace("{{worker_rows}}", &worker_rows)
        .replace("{{user}}", &github_login)
        .replace("{{user_avatar}}", &user_avatar_html))
        .into_response()
}

use crate::rpc::{MasterService, WorkerServiceClient, WsTransport, multiplex};
use tarpc::server::{self, Channel};

#[derive(Clone)]
struct MasterServer {
    scheduler: Arc<Scheduler>,
    worker_id: Uuid,
    _hostname: String,
}

impl MasterService for MasterServer {
    async fn log(self, _: tarpc::context::Context, msg: crate::task::LogMessage) {
        let db = self.scheduler.get_db();
        if let (Some(log_id), Some(prefix)) = (msg.log_id, &msg.prefix) {
            let _ = db.save_log_line(log_id, prefix, msg.hostname.as_deref(), &msg.text).await;
            self.scheduler.send_log(msg);
        }
    }

    async fn report_metrics(self, _: tarpc::context::Context, metrics: crate::task::SystemMetrics) {
        self.scheduler.update_worker_metrics(self.worker_id, metrics).await;
    }

    async fn get_wasm(self, _: tarpc::context::Context, path: String) -> Option<Vec<u8>> {
        self.scheduler.get_wasm_binary(&path).await.map(|arc| (*arc).clone())
    }

    async fn task_started(self, _: tarpc::context::Context, task_id: i64, task_name: String) {
        self.scheduler.update_worker_task(self.worker_id, task_id, task_name, true).await;
        self.scheduler.broadcast_status().await;
    }

    async fn task_result(self, _: tarpc::context::Context, task_id: i64, _log_id: Uuid, _success: bool, _error: Option<String>) {
        self.scheduler.update_worker_task(self.worker_id, task_id, "".to_string(), false).await;
        self.scheduler.broadcast_status().await;
    }

    async fn kv_get(self, _: tarpc::context::Context, task_id: i64, token: String, key: String) -> Option<String> {
        if self.scheduler.check_task_token(task_id, &token).await {
            self.scheduler.get_db().get_app_kv(task_id, &key).await.ok().flatten()
        } else {
            None
        }
    }

    async fn kv_set(self, _: tarpc::context::Context, task_id: i64, token: String, key: String, value: String) {
        if self.scheduler.check_task_token(task_id, &token).await {
            let _ = self.scheduler.get_db().set_app_kv(task_id, &key, &value).await;
        }
    }
}

pub async fn worker_websocket_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(scheduler): State<Arc<Scheduler>>,
) -> impl IntoResponse {
    let api_key = headers.get("X-API-Key").and_then(|h| h.to_str().ok()).unwrap_or_default();
    
    if !scheduler.verify_api_key(api_key) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let hostname = headers.get("X-Worker-Hostname").and_then(|h| h.to_str().ok()).unwrap_or_else(|| "unknown").to_string();
    
    let ip_addr = headers.get("x-forwarded-for")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.split(',').next())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| addr.to_string());

    ws.max_message_size(128 * 1024 * 1024)
      .on_upgrade(move |socket| handle_worker_socket(socket, scheduler, ip_addr, hostname))
}

async fn handle_worker_socket(socket: WebSocket, scheduler: Arc<Scheduler>, addr: String, hostname: String) {
    use futures_util::StreamExt;
    let worker_id = Uuid::new_v4();
    
    // Wrap WebSocket in tarpc-compatible transport
    let transport = WsTransport::new(
        socket,
        |bin| axum::extract::ws::Message::Binary(bin.into()),
        |msg| {
            if let axum::extract::ws::Message::Binary(bin) = msg {
                Some(bin.into())
            } else {
                None
            }
        }
    );
    
    // Multiplex bidirectional RPC
    let (master_transport, worker_transport) = multiplex(transport);

    // Setup Master Server (handles calls FROM worker)
    let master_server = MasterServer {
        scheduler: scheduler.clone(),
        worker_id,
        _hostname: hostname.clone(),
    };

    // Setup Worker Client (allows calling TO worker)
    let worker_client = WorkerServiceClient::new(tarpc::client::Config::default(), worker_transport).spawn();

    let worker_info = crate::task::WorkerInfo {
        id: worker_id,
        addr: addr.clone(),
        hostname: hostname.clone(),
        running_tasks: Vec::new(),
        metrics: None,
    };
    
    scheduler.add_worker(worker_info, worker_client.clone()).await;
    let _registration = scheduler.clone().track_worker(worker_id);

    println!("Worker connected via tarpc/WebSocket: {} ({}). Active workers: {}", hostname, addr, scheduler.num_workers());

    // Send bootstrap info
    let bootstrap_info = scheduler.get_bootstrap_info_rpc().await;
    let _ = worker_client.bootstrap(tarpc::context::current(), bootstrap_info.0, bootstrap_info.1).await;

    // Run the RPC server for this worker connection
    server::BaseChannel::with_defaults(master_transport)
        .execute(master_server.serve())
        .for_each(|f| async move { f.await })
        .await;

    println!("Worker disconnected: {}. Active workers: {}", hostname, scheduler.num_workers() - 1);
}

use tokio_stream::StreamExt as TokioStreamExt;
use std::convert::Infallible;

pub async fn status_page_handler(
    State(scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> impl IntoResponse {
    let tasks = scheduler.get_tasks_status().await;
    
    let mut rows = String::new();
    for task in tasks {
        let mut status_class;
        let mut status_text;
        let mut status_onclick = String::new();

        if let Some(host) = &task.host {
            // Task is currently running on a worker
            status_text = format!("Running on {}", host);
            status_class = "bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-900/20 dark:text-blue-400 dark:border-blue-800".to_string();
            if let Some(log_id) = task.last_log_id {
                status_onclick = format!("onclick='showTaskLogs(\"{}\")'", log_id);
            }
        } else {
            // Task is NOT currently running
            if !task.enabled {
                status_text = "Paused".to_string();
                status_class = "bg-slate-100 text-slate-500 border-slate-200 dark:bg-slate-800 dark:text-slate-400 dark:border-slate-700".to_string();
            } else {
                status_text = if let Some(last_host) = &task.last_host {
                    format!("Idle ({})", last_host)
                } else {
                    "Idle".to_string()
                };
                status_class = "bg-green-50 text-green-700 border-green-200 dark:bg-green-900/20 dark:text-green-400 dark:border-green-800".to_string();
                
                if task.last_failed {
                    status_text = if let Some(last_host) = &task.last_host {
                        format!("Failed ({})", last_host)
                    } else {
                        "Failed".to_string()
                    };
                    status_class = "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/20 dark:text-red-400 dark:border-red-800 cursor-pointer hover:bg-red-100 dark:hover:bg-red-900/40".to_string();
                }
            }
            if let Some(log_id) = task.last_log_id {
                status_onclick = format!("onclick='showTaskLogs(\"{}\")'", log_id);
            }
        }

        let last_run = format_relative_time(&task.last_run);
        let duration = task.last_duration_ms.map(|ms| format!("<span class='ml-1 text-xs text-blue-600 dark:text-blue-400 font-bold'>({}ms)</span>", ms)).unwrap_or_default();
        
        let run_btn = if task.enabled {
            format!("<button class='px-3 py-1 text-xs font-semibold text-blue-600 bg-blue-50 border border-blue-200 rounded-md hover:bg-blue-600 hover:text-white dark:bg-blue-900/20 dark:border-blue-800 dark:hover:bg-blue-600 transition-colors mr-2' onclick='runTask(\"{}\")'>Run</button>", task.id)
        } else {
            String::new()
        };

        let toggle_btn = if task.enabled {
            format!("<button class='px-3 py-1 text-xs font-semibold text-red-600 bg-red-50 border border-red-200 rounded-md hover:bg-red-600 hover:text-white dark:bg-red-900/20 dark:border-red-800 dark:hover:bg-blue-600 transition-colors' onclick='toggleTask(\"{}\", false)'>Disable</button>", task.id)
        } else {
            format!("<button class='px-3 py-1 text-xs font-semibold text-green-600 bg-green-50 border border-green-200 rounded-md hover:bg-green-600 hover:text-white dark:bg-green-900/20 dark:border-green-800 dark:hover:bg-green-600 transition-colors' onclick='toggleTask(\"{}\", true)'>Enable</button>", task.id)
        };

        rows.push_str(&format!(
            "<tr class='border-b border-gray-100 dark:border-slate-800 hover:bg-gray-50 dark:hover:bg-slate-800/50 transition-colors'>
                <td class='px-4 py-3 align-middle text-sm font-bold text-gray-900 dark:text-slate-100 cursor-pointer hover:text-blue-600 dark:hover:text-blue-400' onclick='filterTask(\"{id}\")'>{name}</td>
                <td class='px-4 py-3 align-middle text-xs'><span class='bg-gray-100 dark:bg-slate-800 text-gray-600 dark:text-slate-400 px-2 py-1 rounded font-medium uppercase tracking-wider'>{t_type}</span></td>
                <td class='px-4 py-3 align-middle font-mono text-blue-600 dark:text-blue-400 text-xs'>{cron}</td>
                <td class='px-4 py-3 align-middle text-gray-600 dark:text-slate-400 text-sm'>{tz}</td>
                <td class='px-4 py-3 align-middle text-sm text-gray-500 dark:text-slate-500' title='{raw_run}'>{last} {dur}</td>
                <td class='px-4 py-3 align-middle'><span class='px-2 py-1 text-[10px] uppercase font-bold rounded-full border {s_class}' {s_onclick}>{s_text}</span></td>
                <td class='px-4 py-3 align-middle'>{run_btn}{btn}</td>
            </tr>",
            id = task.id,
            name = task.name,
            t_type = task.task_type,
            cron = task.cron,
            tz = task.timezone,
            last = last_run,
            raw_run = task.last_run.as_deref().unwrap_or("Never"),
            dur = duration,
            s_class = status_class,
            s_onclick = status_onclick,
            s_text = status_text,
            run_btn = run_btn,
            btn = toggle_btn
        ));
    }

    let template = match tokio::fs::read_to_string("web/templates/status.html").await {
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
    axum::extract::Query(params): axum::extract::Query<ClusterLogsParams>,
    session: Session,
) -> impl IntoResponse {
    let page = params.page.unwrap_or(1);
    let limit = 50;
    let offset = (page - 1) * limit;

    let db = scheduler.get_db();
    let webhooks = db.get_webhooks_paginated(limit, offset).await.unwrap_or_default();
    let total_count = db.get_webhooks_count().await.unwrap_or(0);
    let total_pages = (total_count + limit - 1) / limit;
    
    let mut rows = String::new();
    for wh in webhooks {
        rows.push_str(&format!(
            "<tr id='row-{id}' data-headers='{headers}' data-body='{body}' class='border-b border-gray-100 dark:border-slate-800 hover:bg-gray-50 dark:hover:bg-slate-800/50 transition-colors'>
                <td class='px-4 py-3 align-middle text-xs font-mono text-slate-500 dark:text-slate-500' title='{id}'>{id}</td>
                <td class='px-4 py-3 align-middle text-sm text-gray-500 dark:text-slate-500'>{time}</td>
                <td class='px-4 py-3 align-middle'><span class='px-2 py-0.5 bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400 rounded text-[10px] font-bold uppercase'>{method}</span></td>
                <td class='px-4 py-3 align-middle text-sm font-bold text-slate-900 dark:text-slate-100'>{path}</td>
                <td class='px-4 py-3 align-middle text-xs font-mono text-slate-500 dark:text-slate-500'>{remote_addr}</td>
                <td class='px-4 py-3 align-middle text-right space-x-2'>
                    <button class='text-blue-600 hover:text-blue-800 text-xs font-bold' onclick='showDetails({id})'>DETAILS</button>
                    <button class='text-slate-400 hover:text-red-500 transition-colors' onclick='deleteWebhook(\"{id}\")'>
                        <svg class='w-4 h-4 inline' fill='none' stroke='currentColor' viewBox='0 0 24 24'><path stroke-linecap='round' stroke-linejoin='round' stroke-width='2' d='M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16'></path></svg>
                    </button>
                </td>
            </tr>",
            id = wh.id,
            path = wh.path,
            method = wh.method,
            remote_addr = wh.remote_addr.as_deref().unwrap_or("-"),
            time = wh.created_at,
            headers = wh.headers.replace("&", "&amp;").replace("'", "&#39;"),
            body = wh.body.replace("&", "&amp;").replace("'", "&#39;")
        ));
    }

    if rows.is_empty() {
        rows = "<tr><td colspan='6' class='px-4 py-8 text-center text-slate-500 dark:text-slate-400 italic'>No webhooks received yet</td></tr>".to_string();
    }

    let mut pagination_html = String::new();
    if total_pages > 1 {
        pagination_html.push_str("<div class='px-4 py-3 bg-slate-50 dark:bg-slate-800/30 border-t border-slate-100 dark:border-slate-800 flex items-center justify-between'>");
        pagination_html.push_str(&format!(
            "<div class='text-sm text-slate-500 dark:text-slate-400'>Showing <span class='font-medium'>{}</span> to <span class='font-medium'>{}</span> of <span class='font-medium'>{}</span> results</div>",
            if total_count == 0 { 0 } else { offset + 1 },
            std::cmp::min(offset + limit, total_count),
            total_count
        ));
        pagination_html.push_str("<div class='flex gap-2'>");
        
        if page > 1 {
            pagination_html.push_str(&format!("<a href='/webhook-status?page={}' class='px-3 py-1 text-xs font-medium rounded-md border border-slate-200 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-800 transition-colors'>Previous</a>", page - 1));
        }
        
        if page < total_pages {
            pagination_html.push_str(&format!("<a href='/webhook-status?page={}' class='px-3 py-1 text-xs font-medium rounded-md border border-slate-200 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-800 transition-colors'>Next</a>", page + 1));
        }
        
        pagination_html.push_str("</div></div>");
    }

    let template = match tokio::fs::read_to_string("web/templates/webhook_status.html").await {
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
        .replace("{{pagination}}", &pagination_html)
        .replace("{{user}}", &github_login)
        .replace("{{user_avatar}}", &user_avatar_html))
        .into_response()
}

pub async fn events_handler(
    State(scheduler): State<Arc<Scheduler>>,
) -> impl IntoResponse {
    let log_rx = scheduler.subscribe_logs();
    let status_rx = scheduler.subscribe_status();
    let webhook_rx = scheduler.subscribe_webhooks();
    let workers_rx = scheduler.subscribe_workers();

    // Send an initial connected message to ensure headers are flushed and proxies don't buffer
    let initial_msg = futures_util::stream::once(async {
        let json = serde_json::json!({ "connected": true }).to_string();
        Ok::<Event, Infallible>(Event::default().data(json))
    });

    let log_stream = tokio_stream::wrappers::BroadcastStream::new(log_rx)
        .map(|msg| {
            match msg {
                Ok(log_msg) => {
                    let json = serde_json::json!({ 
                        "log": log_msg.text,
                        "task_id": log_msg.task_id,
                        "host": log_msg.hostname.unwrap_or_else(|| "unknown".to_string())
                    }).to_string();
                    Ok(Event::default().data(json))
                },

                Err(_) => {
                    let json = serde_json::json!({ "log": "... (log buffer overflowed)" }).to_string();
                    Ok(Event::default().data(json))
                },
            }
        });

    let status_stream = tokio_stream::wrappers::BroadcastStream::new(status_rx)
        .map(|msg| {
            match msg {
                Ok(status) => {
                    let json = serde_json::json!({ "tasks": status }).to_string();
                    Ok(Event::default().data(json))
                },
                Err(_) => {
                    let json = serde_json::json!({ "log": "... (status buffer overflowed)" }).to_string();
                    Ok(Event::default().data(json))
                },
            }
        });

    let webhook_stream = tokio_stream::wrappers::BroadcastStream::new(webhook_rx)
        .map(|msg| {
            match msg {
                Ok(webhook) => {
                    let json = serde_json::json!({ "webhook": webhook }).to_string();
                    Ok(Event::default().data(json))
                },
                Err(_) => {
                    let json = serde_json::json!({ "log": "... (webhook buffer overflowed)" }).to_string();
                    Ok(Event::default().data(json))
                },
            }
        });

    let workers_stream = tokio_stream::wrappers::BroadcastStream::new(workers_rx)
        .map(|msg| {
            match msg {
                Ok(workers) => {
                    let json = serde_json::json!({ "workers": workers }).to_string();
                    Ok(Event::default().data(json))
                },
                Err(_) => {
                    let json = serde_json::json!({ "log": "... (workers buffer overflowed)" }).to_string();
                    Ok(Event::default().data(json))
                },
            }
        });

    let combined = TokioStreamExt::merge(
        TokioStreamExt::merge(
            TokioStreamExt::merge(
                initial_msg.chain(log_stream), 
                status_stream
            ),
            webhook_stream
        ),
        workers_stream
    );

    let sse = Sse::new(combined).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive")
    );

    (
        [
            ("x-accel-buffering", "no"), // Disable buffering for Nginx
        ],
        sse
    ).into_response()
}

fn format_relative_time(dt_str: &Option<String>) -> String {
    if let Some(s) = dt_str {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            let now = chrono::Utc::now();
            let diff = now.signed_duration_since(dt.with_timezone(&chrono::Utc));
            
            if diff.num_seconds() < 0 {
                return "Just now".to_string();
            } else if diff.num_seconds() < 60 {
                return format!("{}s ago", diff.num_seconds());
            } else if diff.num_minutes() < 60 {
                return format!("{}m ago", diff.num_minutes());
            } else if diff.num_hours() < 24 {
                return format!("{}h ago", diff.num_hours());
            } else {
                return dt.format("%Y-%m-%d %H:%M").to_string();
            }
        }
    }
    "Never".to_string()
}

pub async fn webhook_handler(
    State(scheduler): State<Arc<Scheduler>>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let path = uri.path().to_string();
    let method_str = method.to_string();
    
    // Prioritize X-Forwarded-For header for proxied setups
    let remote_addr = headers
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| addr.ip().to_string());

    let headers_str = format!("{:?}", headers);
    let body_str = String::from_utf8_lossy(&body).to_string();
    
    let db = scheduler.get_db();
    match db.save_webhook(&path, &method_str, Some(&remote_addr), &headers_str, &body_str).await {
        Ok(webhook) => {
            scheduler.broadcast_webhook(webhook);
            StatusCode::OK.into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save webhook: {}", e)).into_response(),
    }
}

pub async fn delete_webhook_handler(
    State(scheduler): State<Arc<Scheduler>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let db = scheduler.get_db();
    match db.delete_webhook(id).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete webhook: {}", e)).into_response(),
    }
}

pub async fn get_run_logs_handler(
    State(scheduler): State<Arc<Scheduler>>,
    Path(log_id): Path<Uuid>,
) -> impl IntoResponse {
    let db = scheduler.get_db();
    match db.get_run_logs(log_id).await {
        Ok(logs) => {
            let json_logs: Vec<serde_json::Value> = logs.into_iter()
                .map(|(module, host, output)| {
                    serde_json::json!({
                        "module": module,
                        "host": host,
                        "output": output
                    })
                })
                .collect();
            Json(json_logs).into_response()
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to fetch logs: {}", e)).into_response(),
    }
}

pub async fn get_tasks_handler(
    State(scheduler): State<Arc<Scheduler>>,
) -> impl IntoResponse {
    let tasks = scheduler.get_tasks_status().await;
    Json(tasks).into_response()
}

pub async fn toggle_task_handler(
    State(scheduler): State<Arc<Scheduler>>,
    Path(task_id): Path<i64>,
    Json(payload): Json<ToggleRequest>,
) -> impl IntoResponse {
    scheduler.set_task_enabled(task_id, payload.enabled).await;
    StatusCode::OK.into_response()
}

pub async fn run_task_handler(
    State(scheduler): State<Arc<Scheduler>>,
    Path(task_id): Path<i64>,
) -> impl IntoResponse {
    match scheduler.run_task_immediately(task_id).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

pub async fn reload_config_handler(
    State(scheduler): State<Arc<Scheduler>>,
) -> impl IntoResponse {
    match scheduler.reload_from_file("cron.toml").await {
        Ok(_) => (StatusCode::OK, "Configuration reloaded successfully").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reload configuration: {}", e)).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ClusterLogsParams {
    pub page: Option<usize>,
}

pub async fn task_logs_page_handler(
    State(_scheduler): State<Arc<Scheduler>>,
    session: Session,
) -> impl IntoResponse {
    let template = match tokio::fs::read_to_string("web/templates/task_logs.html").await {
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
        .replace("{{user}}", &github_login)
        .replace("{{user_avatar}}", &user_avatar_html))
        .into_response()
}

pub async fn get_task_logs_handler(
    State(scheduler): State<Arc<Scheduler>>,
    axum::extract::Query(params): axum::extract::Query<ClusterLogsParams>,
) -> impl IntoResponse {
    let page = params.page.unwrap_or(1);
    let limit = 50;
    let offset = (page - 1) * limit;

    let db = scheduler.get_db();
    let logs = match db.get_task_logs_paginated(limit, offset).await {
        Ok(l) => l,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    };

    let total_count = match db.get_task_logs_count().await {
        Ok(c) => c,
        Err(_) => 0,
    };

    Json(serde_json::json!({
        "logs": logs,
        "total_count": total_count,
        "page": page,
        "limit": limit
    })).into_response()
}

pub async fn task_rpc_handler(
    State(scheduler): State<Arc<Scheduler>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let method = req["method"].as_str().unwrap_or_default();
    let id = req["id"].clone();
    let params = &req["params"];
    
    let task_id = params["task_id"].as_i64().unwrap_or(0);
    let token = params["token"].as_str().unwrap_or_default();

    // Verify token
    if !scheduler.check_task_token(task_id, token).await {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": -32000, "message": "Invalid or expired task token" },
            "id": id
        }))).into_response();
    }

    match method {
        "kv_get" => {
            let key = params["key"].as_str().unwrap_or_default();
            let value = scheduler.get_db().get_app_kv(task_id, key).await.ok().flatten();
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "result": value,
                "id": id
            })).into_response()
        },
        "kv_set" => {
            let key = params["key"].as_str().unwrap_or_default();
            let value = params["value"].as_str().unwrap_or_default();
            let _ = scheduler.get_db().set_app_kv(task_id, key, value).await;
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "result": "ok",
                "id": id
            })).into_response()
        },
        _ => (StatusCode::NOT_FOUND, Json(serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": -32601, "message": "Method not found" },
            "id": id
        }))).into_response()
    }
}
