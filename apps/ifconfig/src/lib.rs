use anyhow::Result;
use spin_sdk::{
    http::{IntoResponse, Request, Response},
    http_component,
};
use std::net::{IpAddr, SocketAddr};

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

    let socket_addr: SocketAddr =
        x_forwarded_for.parse().expect("Invalid socket address");

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body(socket_addr.ip().to_string())
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
