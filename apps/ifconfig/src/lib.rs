use anyhow::Result;
use serde_json::Value;
use spin_sdk::{
    http::{IntoResponse, Method::Get, Request, Response},
    http_component,
};
use std::str;

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    whoyouare(&req).await?;

    let spin_client_addr =
        req.header("spin-client-addr").unwrap().as_str().unwrap();
    let x_forwarded_for = match req.header("x-forwarded-for") {
        Some(x) => x.as_str().unwrap(),
        None => spin_client_addr,
    };

    let (country, city) = match geoip(x_forwarded_for).await? {
        Some(x) => {
            let city = x
                .get("city")
                .unwrap()
                .get("names")
                .unwrap()
                .get("en")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            let country = x
                .get("country")
                .unwrap()
                .get("names")
                .unwrap()
                .get("en")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            (country, city)
        }
        None => ("".to_string(), "".to_string()),
    };

    let txt_output = format!("{}\n{}\n{}", x_forwarded_for, country, city);

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body(txt_output.trim())
        .build())
}

async fn whoyouare(req: &Request) -> Result<()> {
    let headers = req.headers();
    let method = req.method().to_string();
    for header in headers {
        println!("{}: {}", header.0, header.1.as_str().unwrap());
    }
    println!("{:?}", method);
    Ok(())
}

async fn geoip(ip: &str) -> Result<Option<Value>> {
    let uri = format!("http://localhost:3000/geoip?{}", ip);
    // let uri = format!("http://geoip.spin.internal/geoip?{}", ip);
    let request = Request::builder().method(Get).uri(uri).build();
    let response: Response = spin_sdk::http::send(request).await?;

    if response.status() == &406u16 {
        return Ok(None);
    }

    match response.status() {
        200u16 => {
            let response_body = str::from_utf8(response.body()).unwrap();
            let v: Value = serde_json::from_str(response_body).unwrap();
            Ok(Some(v))
        }
        406u16 => Ok(None),
        _ => Ok(None),
    }
}
