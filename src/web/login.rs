use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect},
    http::StatusCode,
};
use tower_sessions::Session;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::scheduler::Scheduler;
use rand::distr::{Alphanumeric, SampleString};

pub const USER_SESSION_KEY: &str = "user_github_id";

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    code: String,
    state: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GitHubUser {
    id: i64,
    login: String,
    avatar_url: String,
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
    State(scheduler): State<Arc<Scheduler>>,
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
        .header("User-Agent", "lachuoi")
        .header("Authorization", format!("Bearer {}", token_data.access_token))
        .send()
        .await {
            Ok(resp) => match resp.json().await {
                Ok(u) => u,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to parse user info: {}", e)).into_response(),
            },
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to fetch user info: {}", e)).into_response(),
        };

    // Authorization check
    let db = scheduler.get_db();
    match db.is_authorized(&user_info.login).await {
        Ok(true) => {
            session.insert(USER_SESSION_KEY, user_info.id).await.unwrap();
            session.insert("github_login", user_info.login).await.unwrap();
            session.insert("github_avatar_url", user_info.avatar_url).await.unwrap();
            Redirect::to("/task-status").into_response()
        }
        Ok(false) => {
            // Not authorized
            let template = std::fs::read_to_string("web/templates/login.html").unwrap_or_default();
            let error_html = template.replace("<!-- ERROR_MESSAGE -->", 
                "<div class='bg-red-50 border border-red-200 text-red-700 px-4 py-3 rounded-lg mb-6 text-sm font-medium'>
                    Access denied. Your GitHub account is not authorized to access this system.
                </div>");
            Html(error_html).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error during authorization: {}", e)).into_response(),
    }
}

pub async fn logout(session: Session) -> impl IntoResponse {
    session.clear().await;
    Redirect::to("/")
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
