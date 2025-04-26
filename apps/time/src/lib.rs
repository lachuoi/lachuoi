use chrono::format::ParseError;
use chrono::{DateTime, NaiveDateTime, Utc};
use percent_encoding::percent_decode;
use serde::Serialize;
use serde_json::json;
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router},
    http_component,
};

#[derive(Serialize)]
struct DateTimeDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    original_timestring: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_timestring_format: Option<String>,
    unix_time: i64,
    rfc2822: String,
    rfc3339: String,
    sql_datetime: String,
}

#[http_component]
async fn handle_root(req: Request) -> anyhow::Result<impl IntoResponse> {
    let mut router = Router::new();
    router.get_async("/time", time);
    router.get_async("/time/now", now);
    router.get_async("/time/parse", convert);
    Ok(router.handle(req))
}

async fn time(
    _req: Request,
    _params: Params,
) -> anyhow::Result<impl IntoResponse> {
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Usage: time/now, time/parse")
        .build())
}

async fn now(
    _req: Request,
    _params: Params,
) -> anyhow::Result<impl IntoResponse> {
    let current_utc = Utc::now();

    let time_description = DateTimeDescription {
        original_timestring: None,
        original_timestring_format: None,
        unix_time: current_utc.timestamp(),
        rfc2822: current_utc.to_rfc2822(),
        rfc3339: current_utc.to_rfc3339().to_string(),
        sql_datetime: current_utc.format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    let b = serde_json::to_string(&time_description);

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(b.unwrap())
        .build())
}

async fn convert(
    req: Request,
    _params: Params,
) -> anyhow::Result<impl IntoResponse> {
    let encoded_query = req.query();
    let query = percent_decode(encoded_query.as_bytes()).decode_utf8_lossy();
    let a = match parse_with_formats(&query).await? {
        Some(a) => a,
        None => {
            return Ok(Response::builder()
                .status(200)
                .header("content-type", "text/plain")
                .body("Not Valid format")
                .build());
        }
    };
    let time_description = DateTimeDescription {
        original_timestring: Some(query.to_string()),
        original_timestring_format: Some(a.1),
        unix_time: a.0.timestamp(),
        rfc2822: a.0.to_rfc2822(),
        rfc3339: a.0.to_rfc3339().to_string(),
        sql_datetime: a.0.format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    let b = serde_json::to_string(&time_description);

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(b.unwrap())
        .build())
}

async fn parse_with_formats(
    timestamp: &str,
) -> anyhow::Result<Option<(DateTime<Utc>, String)>> {
    // List of common timestamp formats to try
    // https://docs.rs/chrono/latest/chrono/format/strftime/index.html

    let formats = [
        "%a, %d %b %Y %H:%M:%S %z", // Thu, 24 Apr 2025 16:28:26 +0000
        "%A, %d %b %Y %H:%M:%S %z", // Thursday, 24 Apr 2025 16:28:26 +0000
        "%a, %d %b %Y %H:%M:%S",    // Thu, 24 Apr 2025 16:28:26 (no timezone)
        "%Y-%m-%dT%H:%M:%S%:z", // 2025-04-24T16:28:26+00:00  <-- use %:z for colon
        "%Y-%m-%dT%H:%M:%S",    // 2025-04-24T16:28:26 (no timezone)
        "%Y-%m-%d %H:%M:%S",    // 2025-04-24 16:28:26
        "%Y-%m-%d %I:%M:%S %p", // 2025-04-24 04:28:26 PM
        "%Y/%m/%d %H:%M:%S",    // 2025/04/24 16:28:26
        "%Y/%m/%d %I:%M:%S %p", // 2025/04/24 04:28:26 PM
        "%m/%d/%Y %H:%M:%S",    // 04/24/2025 16:28:26 (US style)
        "%d/%m/%Y %H:%M:%S",    // 24/04/2025 16:28:26 (European style)
        "%d.%m.%Y %H:%M:%S",    // 24.04.2025 16:28:26
        "%d-%m-%Y %H:%M:%S",    // 24-04-2025 16:28:26
        "%d %b %Y %H:%M:%S",    // 24 Apr 2025 16:28:26
        "%b %d %Y %H:%M:%S",    // Apr 24 2025 16:28:26
        "%a %b %d %H:%M:%S %Y", // Thu Apr 24 16:28:26 2025
    ];
    // First, try parsing with each format
    for &format in &formats {
        if let Ok(parsed) = DateTime::parse_from_str(timestamp, format) {
            return Ok(Some((parsed.with_timezone(&Utc), format.to_string())));
        }
        if let Ok(parsed) = NaiveDateTime::parse_from_str(timestamp, format) {
            return Ok(Some((
                DateTime::from_naive_utc_and_offset(parsed, Utc),
                format.to_string(),
            )));
        }
    }

    // Try parsing as epoch timestamp (number of seconds since 1970-01-01)
    if let Ok(epoch_seconds) = timestamp.parse::<i64>() {
        let naive = DateTime::from_timestamp(epoch_seconds, 0);
        if let Some(naive) = naive {
            return Ok(Some((naive, "UNIX TIME".to_string())));
        }
    }

    Ok(None)
}
