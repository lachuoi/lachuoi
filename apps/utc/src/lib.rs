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
    time_in_rfc2822: String,
}

#[http_component]
async fn handle_root(req: Request) -> anyhow::Result<impl IntoResponse> {
    let encoded_query = req.query();

    if encoded_query.is_empty() {
        let current_utc = Utc::now();

        let time_description = DateTimeDescription {
            original_timestring: None,
            original_timestring_format: None,
            unix_time: current_utc.timestamp(),
            time_in_rfc2822: current_utc.to_rfc2822(),
        };

        let b = serde_json::to_string(&time_description);

        return Ok(Response::builder()
            .status(200)
            .header("content-type", "application/json")
            .body(b.unwrap())
            .build());
    };

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
        time_in_rfc2822: a.0.to_rfc2822(),
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
    let formats = [
        "%a, %d %b %Y %H:%M:%S %z", // e.g., Thu, 24 Apr 2025 16:28:26 +0000
        "%a, %d %b %Y %H:%M:%S", // e.g., Thu, 24 Apr 2025 16:28:26 (without timezone)
        "%Y-%m-%dT%H:%M:%S%z",   // e.g., 2025-04-24T16:28:26+00:00
        "%Y-%m-%dT%H:%M:%S",     // e.g., 2025-04-24T16:28:26 (without timezone)
        "%Y-%m-%d %H:%M:%S",     // e.g., 2025-04-24 16:28:26
        "%Y-%m-%d %I:%M:%S %p",  // e.g., 2025-04-24 04:28:26 PM
        "%Y/%m/%d %H:%M:%S",     // e.g., 2025/04/24 16:28:26
        "%Y/%m/%d %I:%M:%S %p",  // e.g., 2025/04/24 04:28:26 PM
        "%m/%d/%Y %H:%M:%S",     // e.g., 04/24/2025 16:28:26 (US format)
        "%d/%m/%Y %H:%M:%S",     // e.g., 24/04/2025 16:28:26 (European format)
        "%d-%m-%Y %H:%M:%S", // e.g., 24-04-2025 16:28:26 (European format with dashes)
        "%d %b %Y %H:%M:%S", // e.g., 24 Apr 2025 16:28:26
        "%b %d %Y %H:%M:%S", // e.g., Apr 24 2025 16:28:26
        "%a %b %d %H:%M:%S %Y", // e.g., Thu Apr 24 16:28:26 2025
    ];

    // First, try parsing with each format
    for format in &formats {
        if let Ok(parsed) = DateTime::parse_from_str(timestamp, format) {
            return Ok(Some((parsed.with_timezone(&Utc), format.to_string())));
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
