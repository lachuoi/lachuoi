use anyhow::Result;
use maxminddb::geoip2;
use spin_sdk::{
    http::{HeaderValue, IntoResponse, Request, Response},
    http_component,
};
use std::net::IpAddr;
use std::str::FromStr;

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    let reader =
        maxminddb::Reader::open_readfile("GeoLite2-City.mmdb").unwrap();

    let query = req.query();

    let null_string_hv = &HeaderValue::string("".to_string());
    let x_forwarded_host = req
        .header("x-forwarded-host")
        .unwrap_or(null_string_hv)
        .as_str()
        .unwrap();
    let x_forwarded_proto = req
        .header("x-forwarded-proto")
        .unwrap_or(null_string_hv)
        .as_str()
        .unwrap();
    let x_forwarded_for = req
        .header("x-forwarded-for")
        .unwrap_or(null_string_hv)
        .as_str()
        .unwrap();

    if query.is_empty() {
        let message = format!(
            "USAGE: {}://{}{}?{}",
            x_forwarded_proto,
            x_forwarded_host,
            req.path(),
            x_forwarded_for
        );
        return Ok(Response::builder()
            .status(200)
            .header("content-type", "text/plain")
            .body(message)
            .build());
    }

    let ip = match IpAddr::from_str(query) {
        Ok(ip) => ip,
        Err(_) => {
            return Ok(Response::builder()
                .status(406)
                .header("content-type", "text/plain")
                .body("Not valid query")
                .build());
        }
    };
    let city: Option<geoip2::City> = reader.lookup(ip).unwrap();
    let geoip_info = match city {
        Some(x) => serde_json::to_string(&x).unwrap(),
        None => "{}".to_string(),
    };

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(geoip_info)
        .build())
}
