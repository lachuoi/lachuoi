use anyhow::Result;
use openssl::sha;
use openssl::ssl::{SslConnector, SslMethod};
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router},
    http_component,
};
use std::env;

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    println!("Handling request to {:?}", req.header("spin-full-url"));

    let mut hasher = sha::Sha256::new();

    hasher.update(b"Hello, ");
    hasher.update(b"world");

    let hash = hasher.finish();
    println!("Hashed \"Hello, world\" to {}", hex::encode(hash));

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello World!")
        .build())
}
