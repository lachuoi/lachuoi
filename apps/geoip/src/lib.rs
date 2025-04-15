use anyhow::Result;
use maxminddb::geoip2;
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router},
    http_component,
};
use std::env;
use std::net::IpAddr;

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    let reader =
        maxminddb::Reader::open_readfile("GeoLite2-City.mmdb").unwrap();

    let arg = "130.162.154.1";
    let ip: IpAddr = arg.parse().unwrap();
    let city: Option<geoip2::City> = reader.lookup(ip).unwrap();
    println!("{city:#?}");

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello World!")
        .build())
}
